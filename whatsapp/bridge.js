#!/usr/bin/env node
// WhatsApp Web bridge using Baileys.
// Communicates with borg via stdin/stdout JSON lines (NDJSON).
//
// Stdin commands:
//   {"cmd":"send","jid":"...","text":"...","quote_id":"..."}
//   {"cmd":"typing","jid":"..."}
//
// Stdout events:
//   {"event":"qr","data":"..."}
//   {"event":"connected","jid":"..."}
//   {"event":"disconnected","reason":"..."}
//   {"event":"message","jid":"...","id":"...","sender":"...","sender_name":"...","text":"...","timestamp":123,"is_group":true}

import makeWASocket, {
  DisconnectReason,
  useMultiFileAuthState,
  makeCacheableSignalKeyStore,
  getContentType,
} from '@whiskeysockets/baileys';
import pino from 'pino';
import { createInterface } from 'readline';

const AUTH_DIR = process.env.WA_AUTH_DIR || 'whatsapp/auth';
const ASSISTANT_NAME = (process.argv[2] || process.env.ASSISTANT_NAME || 'Borg').toLowerCase();
const logger = pino({ level: 'silent' });

function emit(obj) {
  process.stdout.write(JSON.stringify(obj) + '\n');
}

let sock = null;

async function connect() {
  const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);

  sock = makeWASocket({
    auth: {
      creds: state.creds,
      keys: makeCacheableSignalKeyStore(state.keys, logger),
    },
    logger,
    printQRInTerminal: true,
  });

  sock.ev.on('connection.update', (update) => {
    const { connection, lastDisconnect, qr } = update;

    if (qr) {
      emit({ event: 'qr', data: qr });
    }

    if (connection === 'close') {
      const code = lastDisconnect?.error?.output?.statusCode;
      const reason = lastDisconnect?.error?.message || 'unknown';
      emit({ event: 'disconnected', reason });

      if (code !== DisconnectReason.loggedOut) {
        setTimeout(connect, 3000);
      } else {
        process.exit(0);
      }
    }

    if (connection === 'open') {
      emit({ event: 'connected', jid: sock.user?.id || '' });
    }
  });

  sock.ev.on('creds.update', saveCreds);

  sock.ev.on('messages.upsert', ({ messages, type }) => {
    if (type !== 'notify') return;

    for (const msg of messages) {
      if (!msg.message || msg.key.fromMe) continue;

      const contentType = getContentType(msg.message);
      let text = '';

      if (contentType === 'conversation') {
        text = msg.message.conversation || '';
      } else if (contentType === 'extendedTextMessage') {
        text = msg.message.extendedTextMessage?.text || '';
      } else {
        continue;
      }

      if (!text) continue;

      const jid = msg.key.remoteJid || '';
      const isGroup = jid.endsWith('@g.us');
      const sender = isGroup ? (msg.key.participant || '') : jid;
      const senderName = msg.pushName || sender.split('@')[0];

      const mentionedJids = msg.message.extendedTextMessage?.contextInfo?.mentionedJid || [];
      const selfJid = sock.user?.id || '';
      const mentionsByJid = mentionedJids.some(
        (jid) => selfJid && jid.split('@')[0] === selfJid.split('@')[0]
      );
      const mentionsByName = text.toLowerCase().includes('@' + ASSISTANT_NAME);
      emit({
        event: 'message',
        jid,
        id: msg.key.id || '',
        sender,
        sender_name: senderName,
        text,
        timestamp: msg.messageTimestamp || Math.floor(Date.now() / 1000),
        is_group: isGroup,
        mentions_bot: mentionsByJid || mentionsByName,
      });
    }
  });
}

// Read commands from stdin
const rl = createInterface({ input: process.stdin });
rl.on('line', async (line) => {
  if (!sock) return;
  try {
    const cmd = JSON.parse(line);
    if (cmd.cmd === 'send') {
      const opts = {};
      if (cmd.quote_id) {
        opts.quoted = { key: { remoteJid: cmd.jid, id: cmd.quote_id } };
      }
      await sock.sendMessage(cmd.jid, { text: cmd.text }, opts);
    } else if (cmd.cmd === 'typing') {
      await sock.sendPresenceUpdate('composing', cmd.jid);
    }
  } catch (e) {
    emit({ event: 'error', message: e.message });
  }
});

rl.on('close', () => process.exit(0));

connect().catch((e) => {
  emit({ event: 'error', message: e.message });
  process.exit(1);
});
