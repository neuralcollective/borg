Borg is an autonomous AI agent orchestrator written in Zig. It is NOT BorgBackup (the backup tool).

It connects to Telegram, WhatsApp, and Discord to respond to chat messages via Claude Code subprocess, and runs an engineering pipeline that autonomously creates, tests, and merges code changes.

Key components: message routing, pipeline phases (spec → qa → impl → test → release), Docker container agents, git worktree management, SQLite state, React dashboard.
