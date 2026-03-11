#!/usr/bin/env bun
// Unified messaging sidecar — runs Discord + WhatsApp in a single process.
// Communicates with borg via multiplexed NDJSON over stdin/stdout.
//
// All stdout events include a "source" field: "discord" | "whatsapp"
// All stdin commands include a "target" field: "discord" | "whatsapp"

import { createInterface } from 'readline';

const ASSISTANT_NAME = (process.argv[2] || process.env.ASSISTANT_NAME || 'Borg').toLowerCase();

function emit(source, obj) {
  process.stdout.write(JSON.stringify({ source, ...obj }) + '\n');
}

function splitText(text, limit) {
  if (limit <= 0) return [text];
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
    if (!msg.content && msg.attachments.size === 0) return;

    const mentionsBot = msg.mentions.has(discordClient.user) ||
      (msg.content || '').toLowerCase().includes('@' + ASSISTANT_NAME);

    const attachments = [...msg.attachments.values()].map(a => ({
      url: a.url,
      filename: a.name,
      size: a.size,
      content_type: a.contentType || 'application/octet-stream',
    }));

    emit('discord', {
      event: 'message',
      channel_id: msg.channelId,
      message_id: msg.id,
      sender_id: msg.author.id,
      sender_name: msg.member?.displayName || msg.author.displayName || msg.author.username,
      text: msg.content || '',
      attachments,
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
      try {
        await channel.send({ content: chunks[i], ...opts });
      } catch (e) {
        emit('discord', { event: 'error', channel_id: cmd.channel_id, message: e.message });
      }
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
  let waRetries = 0;

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
        if (code !== DisconnectReason.loggedOut) {
          if (waRetries < 10) {
            const delay = Math.min(1000 * Math.pow(2, waRetries) + Math.random() * 1000, 30000);
            waRetries++;
            setTimeout(connect, delay);
          } else {
            emit('whatsapp', { event: 'error', message: 'max reconnection attempts exceeded' });
          }
        } else {
          emit('whatsapp', { event: 'logged_out', message: 'WhatsApp logged out' });
        }
      }

      if (connection === 'open') {
        waRetries = 0;
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
  try {
    if (cmd.cmd === 'send') {
      const opts = {};
      if (cmd.quote_id) opts.quoted = { key: { remoteJid: cmd.jid, id: cmd.quote_id } };
      await waSock.sendMessage(cmd.jid, { text: cmd.text }, opts);
    } else if (cmd.cmd === 'typing') {
      await waSock.sendPresenceUpdate('composing', cmd.jid);
    }
  } catch (e) {
    emit('whatsapp', { event: 'error', jid: cmd.jid, message: e.message });
  }
}

// ── Slack ────────────────────────────────────────────────────────────────

let slackApp = null;
let slackBotUserId = null;

async function startSlack() {
  const botToken = process.env.SLACK_BOT_TOKEN;
  const appToken = process.env.SLACK_APP_TOKEN;
  if (!botToken || !appToken) return;

  const { App } = await import('@slack/bolt');

  slackApp = new App({
    token: botToken,
    appToken,
    socketMode: true,
    logLevel: 'error',
  });

  slackApp.message(async ({ message, say }) => {
    if (message.bot_id || message.subtype) return;
    const text = message.text || '';
    if (!text.trim()) return;

    const isDm = message.channel_type === 'im';
    const mentionsBot = slackBotUserId
      ? text.includes(`<@${slackBotUserId}>`) || text.toLowerCase().includes('@' + ASSISTANT_NAME)
      : text.toLowerCase().includes('@' + ASSISTANT_NAME);

    emit('slack', {
      event: 'message',
      channel_id: message.channel,
      message_id: message.ts,
      sender_id: message.user || '',
      sender_name: message.user || '',
      text,
      timestamp: Math.floor(Number(message.ts)),
      is_dm: isDm,
      mentions_bot: mentionsBot,
    });
  });

  slackApp.error(async (error) => {
    emit('slack', { event: 'error', message: error.message || String(error) });
  });

  await slackApp.start();

  try {
    const { client } = slackApp;
    const info = await client.auth.test();
    slackBotUserId = info.user_id;
    emit('slack', { event: 'ready', bot_id: info.user_id, bot_name: info.user });
  } catch (e) {
    emit('slack', { event: 'error', message: e.message });
  }
}

async function handleSlackCommand(cmd) {
  if (!slackApp) return;
  const { client } = slackApp;
  try {
    if (cmd.cmd === 'send') {
      const chunks = splitText(cmd.text, 3000);
      for (let i = 0; i < chunks.length; i++) {
        const opts = {
          channel: cmd.channel_id,
          text: chunks[i],
        };
        if (i === 0 && cmd.reply_to) {
          opts.thread_ts = cmd.reply_to;
        }
        await client.chat.postMessage(opts);
      }
    } else if (cmd.cmd === 'typing') {
      // Slack doesn't have a typing indicator API in Socket Mode — no-op
    }
  } catch (e) {
    emit('slack', { event: 'error', channel_id: cmd.channel_id, message: e.message });
  }
}

// ── Per-user Discord bots ────────────────────────────────────────────────

const userDiscordBots = new Map(); // user_id -> { client, botId }

async function addUserDiscordBot(cmd) {
  const { user_id, token } = cmd;
  if (!user_id || !token) return;

  // Remove existing bot for this user if any
  await removeUserDiscordBot({ user_id });

  const { Client, GatewayIntentBits } = await import('discord.js');
  const client = new Client({
    intents: [
      GatewayIntentBits.Guilds,
      GatewayIntentBits.GuildMessages,
      GatewayIntentBits.MessageContent,
      GatewayIntentBits.DirectMessages,
    ],
  });

  client.once('ready', () => {
    userDiscordBots.set(String(user_id), { client, botId: client.user.id });
    emit('discord', { event: 'user_bot_ready', user_id, bot_id: client.user.id, bot_name: client.user.username });
  });

  client.on('messageCreate', (msg) => {
    if (msg.author.bot) return;
    if (!msg.content && msg.attachments.size === 0) return;
    const mentionsBot = msg.mentions.has(client.user) ||
      (msg.content || '').toLowerCase().includes('@' + ASSISTANT_NAME);
    const attachments = [...msg.attachments.values()].map(a => ({
      url: a.url,
      filename: a.name,
      size: a.size,
      content_type: a.contentType || 'application/octet-stream',
    }));
    emit('discord', {
      event: 'message',
      user_id,
      channel_id: msg.channelId,
      message_id: msg.id,
      sender_id: msg.author.id,
      sender_name: msg.member?.displayName || msg.author.displayName || msg.author.username,
      text: msg.content || '',
      attachments,
      timestamp: Math.floor(msg.createdTimestamp / 1000),
      is_dm: !msg.guild,
      mentions_bot: mentionsBot,
    });
  });

  client.on('error', (err) => {
    emit('discord', { event: 'error', user_id, message: err.message });
  });

  await client.login(token).catch((err) => {
    emit('discord', { event: 'error', user_id, message: err.message });
  });
}

async function removeUserDiscordBot(cmd) {
  const key = String(cmd.user_id);
  const bot = userDiscordBots.get(key);
  if (bot) {
    bot.client.destroy();
    userDiscordBots.delete(key);
    emit('discord', { event: 'user_bot_removed', user_id: cmd.user_id });
  }
}

async function handleUserDiscordCommand(cmd) {
  const key = String(cmd.user_id);
  const bot = userDiscordBots.get(key);
  if (!bot) return;
  if (cmd.cmd === 'send') {
    const channel = await bot.client.channels.fetch(cmd.channel_id).catch(() => null);
    if (!channel?.isTextBased()) return;
    const chunks = splitText(cmd.text, 2000);
    for (let i = 0; i < chunks.length; i++) {
      const opts = {};
      if (i === 0 && cmd.reply_to) {
        opts.reply = { messageReference: cmd.reply_to, failIfNotExists: false };
      }
      try {
        await channel.send({ content: chunks[i], ...opts });
      } catch (e) {
        emit('discord', { event: 'error', user_id: cmd.user_id, channel_id: cmd.channel_id, message: e.message });
      }
    }
  } else if (cmd.cmd === 'typing') {
    const channel = await bot.client.channels.fetch(cmd.channel_id).catch(() => null);
    if (channel?.isTextBased()) await channel.sendTyping().catch(() => {});
  }
}

// ── Stdin Router ────────────────────────────────────────────────────────

const rl = createInterface({ input: process.stdin });
rl.on('line', async (line) => {
  let cmd;
  try {
    cmd = JSON.parse(line);
    if (cmd.target === 'discord') {
      if (cmd.cmd === 'add_user_bot') await addUserDiscordBot(cmd);
      else if (cmd.cmd === 'remove_user_bot') await removeUserDiscordBot(cmd);
      else if (cmd.user_id) await handleUserDiscordCommand(cmd);
      else await handleDiscordCommand(cmd);
    }
    else if (cmd.target === 'whatsapp') await handleWhatsAppCommand(cmd);
    else if (cmd.target === 'slack') await handleSlackCommand(cmd);
  } catch (e) {
    emit('system', { event: 'error', target: cmd?.target, message: e.message, stack: e.stack });
  }
});

rl.on('close', async () => {
  if (discordClient) discordClient.destroy();
  for (const bot of userDiscordBots.values()) bot.client.destroy();
  userDiscordBots.clear();
  if (waSock?.ws) waSock.ws.close();
  if (slackApp) await slackApp.stop().catch(() => {});
  process.exit(0);
});

// ── Start ───────────────────────────────────────────────────────────────

await Promise.all([startDiscord(), startWhatsApp(), startSlack()]);
