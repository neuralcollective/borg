#!/usr/bin/env node
// Discord bridge using discord.js.
// Communicates with borg via stdin/stdout JSON lines (NDJSON).
//
// Stdin commands:
//   {"cmd":"send","channel_id":"...","text":"...","reply_to":"..."}
//   {"cmd":"typing","channel_id":"..."}
//
// Stdout events:
//   {"event":"ready","bot_id":"...","bot_name":"..."}
//   {"event":"message","channel_id":"...","message_id":"...","sender_id":"...","sender_name":"...","text":"...","timestamp":123,"is_dm":false,"mentions_bot":false}
//   {"event":"error","message":"..."}

import { Client, GatewayIntentBits } from 'discord.js';
import { createInterface } from 'readline';

const TOKEN = process.env.DISCORD_TOKEN;
const ASSISTANT_NAME = (process.argv[2] || process.env.ASSISTANT_NAME || 'Borg').toLowerCase();

if (!TOKEN) {
  process.stderr.write('DISCORD_TOKEN not set\n');
  process.exit(1);
}

function emit(obj) {
  process.stdout.write(JSON.stringify(obj) + '\n');
}

const client = new Client({
  intents: [
    GatewayIntentBits.Guilds,
    GatewayIntentBits.GuildMessages,
    GatewayIntentBits.MessageContent,
    GatewayIntentBits.DirectMessages,
  ],
});

client.once('ready', () => {
  emit({ event: 'ready', bot_id: client.user.id, bot_name: client.user.username });
});

client.on('messageCreate', (msg) => {
  if (msg.author.bot) return;
  if (!msg.content) return;

  const mentionsBot = msg.mentions.has(client.user) ||
    msg.content.toLowerCase().includes('@' + ASSISTANT_NAME);

  emit({
    event: 'message',
    channel_id: msg.channelId,
    message_id: msg.id,
    sender_id: msg.author.id,
    sender_name: msg.member?.displayName || msg.author.displayName || msg.author.username,
    text: msg.content,
    timestamp: Math.floor(msg.createdTimestamp / 1000),
    is_dm: !msg.guild,
    mentions_bot: mentionsBot,
  });
});

client.on('error', (err) => {
  emit({ event: 'error', message: err.message });
});

// Read commands from stdin
const rl = createInterface({ input: process.stdin });
rl.on('line', async (line) => {
  try {
    const cmd = JSON.parse(line);
    if (cmd.cmd === 'send') {
      const channel = await client.channels.fetch(cmd.channel_id).catch(() => null);
      if (!channel?.isTextBased()) return;
      const chunks = splitText(cmd.text, 2000);
      for (let i = 0; i < chunks.length; i++) {
        const opts = {};
        if (i === 0 && cmd.reply_to) {
          opts.reply = { messageReference: cmd.reply_to, failIfNotExists: false };
        }
        await channel.send({ content: chunks[i], ...opts });
      }
    } else if (cmd.cmd === 'typing') {
      const channel = await client.channels.fetch(cmd.channel_id).catch(() => null);
      if (channel?.isTextBased()) await channel.sendTyping().catch(() => {});
    }
  } catch (e) {
    emit({ event: 'error', message: e.message });
  }
});

rl.on('close', () => {
  client.destroy();
  process.exit(0);
});

function splitText(text, limit) {
  if (text.length <= limit) return [text];
  const chunks = [];
  let remaining = text;
  while (remaining.length > limit) {
    let cut = remaining.lastIndexOf('\n', limit);
    if (cut <= 0) cut = limit;
    chunks.push(remaining.slice(0, cut));
    remaining = remaining.slice(cut).replace(/^\n/, '');
  }
  if (remaining) chunks.push(remaining);
  return chunks;
}

client.login(TOKEN).catch((err) => {
  emit({ event: 'error', message: err.message });
  process.exit(1);
});
