#!/usr/bin/env bun
// Unified messaging sidecar — runs Discord + WhatsApp in a single process.
// Communicates with borg via multiplexed NDJSON over stdin/stdout.
//
// All stdout events include a "source" field: "discord" | "whatsapp"
// All stdin commands include a "target" field: "discord" | "whatsapp"

import { createInterface } from 'readline';
import { spawn } from 'child_process';

const ASSISTANT_NAME = (process.argv[2] || process.env.ASSISTANT_NAME || 'Borg').toLowerCase();

function emit(source, obj) {
  process.stdout.write(JSON.stringify({ source, ...obj }) + '\n');
}

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

// ── Discord ─────────────────────────────────────────────────────────────

let discordClient = null;

async function startDiscord() {
  const token = process.env.DISCORD_TOKEN;
  if (!token) return;

  const { Client, GatewayIntentBits } = await import('discord.js');

  discordClient = new Client({
    intents: [
      GatewayIntentBits.Guilds,
      GatewayIntentBits.GuildMessages,
      GatewayIntentBits.MessageContent,
      GatewayIntentBits.DirectMessages,
    ],
  });

  discordClient.once('ready', () => {
    emit('discord', { event: 'ready', bot_id: discordClient.user.id, bot_name: discordClient.user.username });
  });

  discordClient.on('messageCreate', (msg) => {
    if (msg.author.bot) return;
    if (!msg.content) return;

    const mentionsBot = msg.mentions.has(discordClient.user) ||
      msg.content.toLowerCase().includes('@' + ASSISTANT_NAME);

    emit('discord', {
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

  discordClient.on('error', (err) => {
    emit('discord', { event: 'error', message: err.message });
  });

  await discordClient.login(token).catch((err) => {
    emit('discord', { event: 'error', message: err.message });
  });
}

async function handleDiscordCommand(cmd) {
  if (!discordClient) return;
  if (cmd.cmd === 'send') {
    const channel = await discordClient.channels.fetch(cmd.channel_id).catch(() => null);
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
    const channel = await discordClient.channels.fetch(cmd.channel_id).catch(() => null);
    if (channel?.isTextBased()) await channel.sendTyping().catch(() => {});
  }
}

// ── WhatsApp ────────────────────────────────────────────────────────────

let waSock = null;

async function startWhatsApp() {
  if (process.env.WA_DISABLED === 'true') return;

  const baileys = await import('@whiskeysockets/baileys');
  const makeWASocket = baileys.default;
  const { DisconnectReason, useMultiFileAuthState, makeCacheableSignalKeyStore, getContentType } = baileys;
  const pino = (await import('pino')).default;

  const AUTH_DIR = process.env.WA_AUTH_DIR || 'whatsapp/auth';
  const logger = pino({ level: 'silent' });

  async function connect() {
    const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);

    waSock = makeWASocket({
      auth: {
        creds: state.creds,
        keys: makeCacheableSignalKeyStore(state.keys, logger),
      },
      logger,
      printQRInTerminal: true,
    });

    waSock.ev.on('connection.update', (update) => {
      const { connection, lastDisconnect, qr } = update;

      if (qr) emit('whatsapp', { event: 'qr', data: qr });

      if (connection === 'close') {
        const code = lastDisconnect?.error?.output?.statusCode;
        const reason = lastDisconnect?.error?.message || 'unknown';
        emit('whatsapp', { event: 'disconnected', reason });
        if (code !== DisconnectReason.loggedOut) setTimeout(connect, 3000);
        else process.exit(0);
      }

      if (connection === 'open') {
        emit('whatsapp', { event: 'connected', jid: waSock.user?.id || '' });
      }
    });

    waSock.ev.on('creds.update', saveCreds);

    waSock.ev.on('messages.upsert', ({ messages, type }) => {
      if (type !== 'notify') return;
      for (const msg of messages) {
        if (!msg.message || msg.key.fromMe) continue;
        const contentType = getContentType(msg.message);
        let text = '';
        if (contentType === 'conversation') text = msg.message.conversation || '';
        else if (contentType === 'extendedTextMessage') text = msg.message.extendedTextMessage?.text || '';
        else continue;
        if (!text) continue;

        const jid = msg.key.remoteJid || '';
        const isGroup = jid.endsWith('@g.us');
        const sender = isGroup ? (msg.key.participant || '') : jid;
        const senderName = msg.pushName || sender.split('@')[0];
        const mentionedJids = msg.message.extendedTextMessage?.contextInfo?.mentionedJid || [];
        const selfJid = waSock.user?.id || '';
        const mentionsByJid = mentionedJids.some((j) => selfJid && j.split('@')[0] === selfJid.split('@')[0]);
        const mentionsByName = text.toLowerCase().includes('@' + ASSISTANT_NAME);

        emit('whatsapp', {
          event: 'message',
          jid, id: msg.key.id || '', sender, sender_name: senderName,
          text, timestamp: msg.messageTimestamp || Math.floor(Date.now() / 1000),
          is_group: isGroup, mentions_bot: mentionsByJid || mentionsByName,
        });
      }
    });
  }

  await connect().catch((e) => {
    emit('whatsapp', { event: 'error', message: e.message });
  });
}

async function handleWhatsAppCommand(cmd) {
  if (!waSock) return;
  if (cmd.cmd === 'send') {
    const opts = {};
    if (cmd.quote_id) opts.quoted = { key: { remoteJid: cmd.jid, id: cmd.quote_id } };
    await waSock.sendMessage(cmd.jid, { text: cmd.text }, opts);
  } else if (cmd.cmd === 'typing') {
    await waSock.sendPresenceUpdate('composing', cmd.jid);
  }
}

// ── Agent session manager ────────────────────────────────────────────────

const agentSessions = new Map(); // session_id → { process }

function handleAgentCommand(cmd) {
  const { action, session_id } = cmd;

  if (action === 'start') {
    startAgentSession(session_id, cmd);
  } else if (action === 'inject') {
    // Injection mid-run is not yet supported — log it
    emit('agent', { event: 'inject_queued', session_id, message: cmd.message });
  } else if (action === 'interrupt') {
    const sess = agentSessions.get(session_id);
    if (sess) {
      sess.process.kill('SIGTERM');
      agentSessions.delete(session_id);
      emit('agent', { event: 'interrupted', session_id });
    }
  }
}

function startAgentSession(session_id, cmd) {
  const { instruction, model, oauth_token, worktree_path, session_dir, allowed_tools, resume_session } = cmd;

  const args = [
    '--output-format', 'stream-json',
    '--model', model || 'claude-opus-4-5',
    '--allowedTools', allowed_tools || 'Read,Glob,Grep,Write,Edit,Bash',
    '--max-turns', '200',
  ];

  if (resume_session) {
    args.push('--resume', resume_session);
  }

  args.push('--print', instruction);

  const env = {
    ...process.env,
    HOME: session_dir || process.env.HOME,
    ANTHROPIC_API_KEY: oauth_token || process.env.ANTHROPIC_API_KEY || '',
    CLAUDE_CODE_OAUTH_TOKEN: oauth_token || process.env.CLAUDE_CODE_OAUTH_TOKEN || '',
  };

  const proc = spawn('claude', args, {
    cwd: worktree_path || process.cwd(),
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  agentSessions.set(session_id, { process: proc });

  let outputLines = [];

  proc.stdout.on('data', (data) => {
    const lines = data.toString().split('\n').filter(l => l.trim());
    for (const line of lines) {
      outputLines.push(line);
      emit('agent', { event: 'stream_line', session_id, line });
    }
  });

  proc.stderr.on('data', (data) => {
    const lines = data.toString().split('\n').filter(l => l.trim());
    for (const line of lines) {
      emit('agent', { event: 'stderr', session_id, line });
    }
  });

  proc.on('close', (code) => {
    agentSessions.delete(session_id);

    let output = '';
    let new_session_id = null;
    for (const line of outputLines) {
      try {
        const obj = JSON.parse(line);
        if (obj.type === 'result' && obj.result) output = obj.result;
        if (obj.type === 'system' && obj.session_id) new_session_id = obj.session_id;
        if (obj.type === 'result' && obj.session_id) new_session_id = obj.session_id;
      } catch {}
    }

    emit('agent', { event: 'complete', session_id, output, new_session_id, exit_code: code ?? 0 });
  });

  proc.on('error', (err) => {
    agentSessions.delete(session_id);
    emit('agent', { event: 'error', session_id, message: err.message });
  });
}

// ── Stdin Router ────────────────────────────────────────────────────────

const rl = createInterface({ input: process.stdin });
rl.on('line', async (line) => {
  try {
    const cmd = JSON.parse(line);
    if (cmd.target === 'discord') await handleDiscordCommand(cmd);
    else if (cmd.target === 'whatsapp') await handleWhatsAppCommand(cmd);
    else if (cmd.target === 'agent') handleAgentCommand(cmd);
  } catch (e) {
    emit('system', { event: 'error', message: e.message });
  }
});

rl.on('close', () => {
  if (discordClient) discordClient.destroy();
  process.exit(0);
});

// ── Start ───────────────────────────────────────────────────────────────

await Promise.all([startDiscord(), startWhatsApp()]);
