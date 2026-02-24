# Borg Architecture

Autonomous AI agent orchestrator. Chat messages trigger Claude Code subprocesses. The engineering pipeline runs agents in Docker containers with git worktree isolation. Written in Zig.

## System Overview

```mermaid
graph TB
    subgraph Messaging
        TG[Telegram Bot API]
        WA[WhatsApp Web<br/>Node.js bridge]
    end

    subgraph "borg (Zig binary)"
        ML[Main Loop<br/>non-blocking poll]
        SM[Per-Group<br/>State Machine]
        GM[GroupManager<br/>mutex-protected]
        CMD[Command Handler<br/>/register /task /status]

        subgraph "Chat Path"
            AT[Agent Thread<br/>std.Thread.spawn]
            CC[claude subprocess<br/>--print --stream-json]
        end

        subgraph "Pipeline Path"
            PT[Pipeline Thread<br/>continuous loop]
            SEED[Seeder Agent<br/>discovers tasks]
            MICRO[Micro-loop<br/>spec→qa→impl→test]
            MACRO[Macro-loop<br/>release train]
            REBASE[Rebase Phase<br/>self-healing]
            DC[Docker Container<br/>isolated execution]
        end

        DB[(SQLite<br/>WAL mode)]
    end

    TG -->|long-poll 2s| ML
    WA -->|NDJSON stdout| ML
    ML --> SM
    ML --> CMD
    SM --> GM
    GM -->|spawn| AT
    AT --> CC
    CC -->|NDJSON| AT
    AT -->|result| GM

    PT --> SEED
    PT --> MICRO
    PT --> MACRO
    PT --> REBASE
    MICRO --> DC
    REBASE --> DC
    DC -->|NDJSON| PT

    ML --> DB
    PT --> DB
    AT --> DB

    GM -->|send response| TG
    GM -->|send response| WA
    PT -->|notify| TG
```

## Module Map

```mermaid
graph LR
    subgraph "Core"
        main[main.zig<br/>orchestrator]
        config[config.zig<br/>env + .env]
        db[db.zig<br/>schema + CRUD]
        sqlite[sqlite.zig<br/>C bindings]
    end

    subgraph "Transport"
        tg[telegram.zig<br/>Bot API client]
        wa[whatsapp.zig<br/>bridge wrapper]
        bridge[bridge.js<br/>Baileys + NDJSON]
        http[http.zig<br/>HTTP + Unix socket]
    end

    subgraph "Execution"
        agent[agent.zig<br/>subprocess runner]
        docker[docker.zig<br/>container mgmt]
        pipeline[pipeline.zig<br/>autonomous engine]
        git[git.zig<br/>CLI wrapper]
    end

    subgraph "Util"
        json[json.zig<br/>parse + escape]
    end

    main --> config
    main --> db
    main --> tg
    main --> wa
    main --> agent
    main --> pipeline

    db --> sqlite
    tg --> http
    docker --> http
    pipeline --> docker
    pipeline --> agent
    pipeline --> git
    pipeline --> db
    agent --> json
    wa --> json
    wa -.->|spawns| bridge
```

## Chat Agent Flow

### Per-Group State Machine

```mermaid
stateDiagram-v2
    [*] --> IDLE

    IDLE --> COLLECTING : @mention detected<br/>+ rate limit OK
    IDLE --> IDLE : no mention / rate limited

    COLLECTING --> COLLECTING : more messages arrive<br/>(extend window)
    COLLECTING --> RUNNING : 3s window expires<br/>→ spawn agent thread

    RUNNING --> COOLDOWN : agent completes<br/>→ deliver response
    RUNNING --> IDLE : agent timeout<br/>→ kill process

    COOLDOWN --> IDLE : 5s expires

    note right of COLLECTING
        Messages batched into
        single prompt. Window
        extends on new messages.
    end note

    note right of RUNNING
        Messages arriving during
        this phase are stored in DB
        for the next invocation.
    end note
```

### Message Processing

```mermaid
sequenceDiagram
    participant U as User
    participant TG as Telegram API
    participant ML as Main Loop
    participant GM as GroupManager
    participant DB as SQLite
    participant TH as Agent Thread
    participant CL as claude CLI

    U->>TG: "@Borg what's 2+2?"
    TG->>ML: getUpdates() returns message
    ML->>DB: storeMessage()
    ML->>GM: onTrigger(group_jid)

    alt Rate limit OK + phase is IDLE
        GM->>GM: phase = COLLECTING<br/>deadline = now + 3s
    else Rate limited or not IDLE
        GM-->>ML: ignored (queued for next run)
    end

    Note over ML: 3s collection window...
    U->>TG: "also check my code"
    TG->>ML: getUpdates()
    ML->>DB: storeMessage()
    ML->>GM: extendCollection()

    Note over ML: Window expires
    ML->>GM: getExpiredCollections()
    GM->>GM: phase = RUNNING

    ML->>DB: getMessagesSince(last_timestamp)
    ML->>TH: spawn thread with prompt

    TH->>CL: claude --print --stream-json<br/>stdin: formatted prompt
    CL-->>TH: NDJSON stream
    TH->>TH: parseNdjson() → result text + session_id

    TH->>DB: setSession(session_id)
    TH->>GM: setOutcome(result)

    ML->>GM: getCompletedAgents()
    GM->>GM: phase = COOLDOWN
    ML->>TG: sendMessage(response)

    Note over GM: 5s cooldown
    GM->>GM: phase = IDLE
```

### Prompt Format

```
You are Borg. You always refer to yourself using plural pronouns
(we/us/our, never I/me/my). You are a collective. Respond naturally
and concisely.

Recent messages:
[2024-01-01T12:00:00Z] Alice: Hey @Borg what's the status?
[2024-01-01T12:00:01Z] Bob: yeah check the deploy
[2024-01-01T12:00:03Z] Borg (you): We checked — deploy looks clean.
[2024-01-01T12:00:15Z] Alice: @Borg can you also check logs?

Respond to the latest message. Be concise.
```

## Pipeline Flow

### Task Lifecycle

```mermaid
stateDiagram-v2
    [*] --> backlog : /task or seeder

    backlog --> spec : create worktree + branch
    spec --> qa : manager writes spec.md
    qa --> impl : QA writes test files
    impl --> done : tests pass ✓
    impl --> retry : tests fail ✗
    retry --> impl : attempt < max
    retry --> failed : attempts exhausted

    done --> merged : release train merges
    done --> rebase : merge conflict or<br/>integration tests fail

    rebase --> done : rebase + tests pass ✓
    rebase --> rebase : fix + retry
    rebase --> failed : rebase attempts exhausted

    merged --> [*]
    failed --> [*]

    note right of rebase
        Self-healing: worker agent
        resolves conflicts, rebases
        onto main, re-runs tests,
        re-queues for release train
    end note
```

### Micro-loop (Per Task)

```mermaid
sequenceDiagram
    participant PL as Pipeline Thread
    participant DB as SQLite
    participant GIT as Git
    participant MGR as Manager Agent
    participant QA as QA Agent
    participant WRK as Worker Agent
    participant TST as Test Runner

    PL->>DB: getNextPipelineTask()
    DB-->>PL: task (status: backlog)

    rect rgb(40, 40, 60)
        Note over PL,GIT: Setup Phase
        PL->>GIT: fetch origin main
        PL->>GIT: worktree add .worktrees/task-{id}<br/>-b feature/task-{id}
        PL->>DB: status = "spec"
    end

    rect rgb(40, 60, 40)
        Note over PL,MGR: Spec Phase
        PL->>MGR: "Read codebase, write spec.md"<br/>Tools: Read, Glob, Grep, Write
        MGR-->>PL: spec.md written in worktree
        PL->>GIT: add -A && commit "spec: ..."
        PL->>DB: status = "qa"
    end

    rect rgb(60, 40, 40)
        Note over PL,QA: QA Phase
        PL->>QA: "Read spec.md, write tests"<br/>Tools: Read, Glob, Grep, Write
        QA-->>PL: test files written
        PL->>GIT: add -A && commit "test: ..."
        PL->>DB: status = "impl"
    end

    rect rgb(40, 50, 60)
        Note over PL,TST: Implementation Phase
        PL->>WRK: "Read spec + tests, implement"<br/>Tools: Read, Glob, Grep, Write, Edit, Bash
        WRK-->>PL: implementation written
        PL->>GIT: add -A && commit "impl: ..."
        PL->>TST: run PIPELINE_TEST_CMD in worktree
    end

    alt Tests pass
        PL->>DB: status = "done"
        PL->>DB: enqueueForIntegration(branch)
        PL->>GIT: remove worktree
        PL-->>PL: notify user: "queued for release"
    else Tests fail (attempt < max)
        PL->>DB: status = "retry", store stderr
        PL->>DB: incrementAttempt()
        Note over PL: Next tick retries impl<br/>with error context in prompt
    else Tests fail (attempts exhausted)
        PL->>DB: status = "failed"
        PL->>GIT: remove worktree
    end
```

### Macro-loop (Release Train)

```mermaid
sequenceDiagram
    participant PL as Pipeline Thread
    participant DB as SQLite
    participant GIT as Git
    participant TST as Test Runner
    participant TG as Telegram

    Note over PL: Every RELEASE_INTERVAL_MINS<br/>(default: 180)

    PL->>DB: getQueuedBranches()
    DB-->>PL: [branch-1, branch-2, branch-3]

    PL->>TG: "Release train starting..."

    PL->>GIT: checkout main && pull
    PL->>GIT: checkout -b release-candidate main

    loop For each queued branch
        PL->>GIT: merge --no-ff branch-N

        alt Merge conflict
            PL->>GIT: merge --abort
            PL->>DB: task status = "rebase"
            PL->>DB: resetTaskAttempt()
            PL->>TG: "Task #N has conflicts — rebasing"
        else Merge OK
            PL->>TST: run tests on release-candidate
            alt Tests pass
                PL->>DB: queue status = "merged"
                PL->>DB: task status = "merged"
                PL->>TG: "Task #N merged to main"
            else Tests fail
                PL->>GIT: reset --hard HEAD~1
                PL->>DB: task status = "rebase"
                PL->>DB: resetTaskAttempt()
                PL->>TG: "Task #N failed tests — rebasing"
            end
        end
    end

    alt Any branches merged
        PL->>GIT: checkout main
        PL->>GIT: merge release-candidate
        PL->>GIT: push origin main
        PL->>GIT: delete release-candidate
        PL->>TG: Release digest (merged + excluded)
    else Nothing merged
        PL->>GIT: checkout main
        PL->>GIT: delete release-candidate
        PL->>TG: "No branches merged"
    end
```

### Self-Healing Rebase

```mermaid
flowchart TD
    A[Task excluded from<br/>release train] --> B{Worktree exists?}
    B -->|No| C[Recreate worktree<br/>from branch]
    B -->|Yes| D[git fetch origin]
    C --> D

    D --> E[git rebase origin/main]
    E --> F{Conflicts?}

    F -->|No| G[Run tests]
    F -->|Yes| H[git rebase --abort]

    H --> I[Spawn worker agent:<br/>rebase + resolve conflicts<br/>+ run tests]
    I --> G

    G --> J{Tests pass?}

    J -->|Yes| K[git push --force-with-lease]
    K --> L[Re-queue for<br/>integration]
    L --> M[Clean up worktree]
    M --> N[Notify user:<br/>rebased + re-queued]

    J -->|No| O{Attempts < max?}
    O -->|Yes| P[Store error, increment attempt]
    P --> Q[Stay in rebase status<br/>next tick retries]
    O -->|No| R[Mark failed<br/>clean up worktree]
```

### Auto-Seeding

```mermaid
flowchart TD
    A[Pipeline tick] --> B{Any active tasks?}
    B -->|Yes| C[Process next task<br/>normal micro-loop]
    B -->|No| D{Seed cooldown<br/>elapsed? >1h}

    D -->|No| E[Sleep 30s]
    D -->|Yes| F{Active tasks<br/>< 5?}

    F -->|No| E
    F -->|Yes| G[Spawn manager agent<br/>against target repo]

    G --> H[Agent analyzes codebase:<br/>find 1-3 refactoring tasks]

    H --> I[Parse TASK_START/TASK_END<br/>blocks from output]

    I --> J[Create pipeline tasks<br/>in DB as 'backlog']

    J --> K[Log + notify admin]
    K --> E

    style G fill:#345,stroke:#6af
    style H fill:#345,stroke:#6af
```

## Database Schema

```mermaid
erDiagram
    registered_groups {
        TEXT jid PK "tg:123 or wa:jid@g.us"
        TEXT name
        TEXT folder "unique per group"
        TEXT trigger_pattern "@Borg"
        INTEGER requires_trigger "1=yes"
        TEXT added_at
    }

    messages {
        TEXT id PK
        TEXT chat_jid PK
        TEXT sender
        TEXT sender_name
        TEXT content
        TEXT timestamp
        INTEGER is_from_me
        INTEGER is_bot_message
    }

    sessions {
        TEXT folder PK
        TEXT session_id "claude --resume ID"
        TEXT created_at
    }

    pipeline_tasks {
        INTEGER id PK
        TEXT title
        TEXT description
        TEXT repo_path
        TEXT branch "feature/task-{id}"
        TEXT status "backlog|spec|qa|impl|retry|done|rebase|failed|merged"
        INTEGER attempt "current retry count"
        INTEGER max_attempts "default 3"
        TEXT last_error
        TEXT created_by "user or seeder"
        TEXT notify_chat "chat JID for updates"
        TEXT created_at
        TEXT updated_at
    }

    integration_queue {
        INTEGER id PK
        INTEGER task_id FK
        TEXT branch
        TEXT status "queued|merging|merged|excluded"
        TEXT error_msg
        TEXT queued_at
    }

    state {
        TEXT key PK
        TEXT value
    }

    registered_groups ||--o{ messages : "chat_jid"
    registered_groups ||--o| sessions : "folder"
    pipeline_tasks ||--o{ integration_queue : "task_id"
```

## Thread Model

```mermaid
graph TB
    subgraph "Main Thread"
        ML[Main Loop<br/>polls TG + WA<br/>checks state machine<br/>delivers results]
    end

    subgraph "Agent Threads (up to MAX_CONCURRENT_AGENTS)"
        A1[Agent Thread 1<br/>group: tg:123]
        A2[Agent Thread 2<br/>group: wa:abc@g.us]
        A3[Agent Thread 3<br/>group: tg:456]
    end

    subgraph "Pipeline Thread"
        PT[Pipeline Loop<br/>tick every 30s]
    end

    subgraph "Shared State"
        GM[GroupManager<br/>std.Thread.Mutex]
        DB[(SQLite<br/>WAL + busy_timeout)]
        SD[shutdown_requested<br/>std.atomic.Value]
    end

    ML --> GM
    A1 --> GM
    A2 --> GM
    A3 --> GM

    ML --> DB
    A1 --> DB
    A2 --> DB
    A3 --> DB
    PT --> DB

    ML --> SD
    PT --> SD

    style GM fill:#644,stroke:#f66
    style SD fill:#446,stroke:#66f
```

**Synchronization rules:**
- `GroupManager.mu` (mutex) held only for brief state transitions (~microseconds)
- Agent execution runs entirely outside the lock
- `AgentContext` is heap-allocated with duped strings — safe for thread lifetime
- `AgentOutcome` is heap-allocated by agent thread, read + freed by main loop
- SQLite WAL mode + `busy_timeout=5000ms` handles concurrent reads/writes
- `shutdown_requested` is `std.atomic.Value(bool)` — lock-free

## Container Security (Pipeline Only)

```mermaid
graph TD
    subgraph "Host"
        REPO[Target Repo<br/>bind-mounted read-write]
        OAUTH[OAuth Token<br/>env var only]
        DOCKER[Docker Daemon<br/>Unix socket]
    end

    subgraph "borg-agent Container"
        AGENT[Claude Code CLI]
        WT[/workspace/repo<br/>worktree mount]
    end

    REPO -->|bind mount| WT
    OAUTH -->|CLAUDE_CODE_OAUTH_TOKEN| AGENT
    DOCKER -->|create + start| AGENT

    subgraph "Security Constraints"
        S1[--cap-drop ALL]
        S2[--no-new-privileges]
        S3[--pids-limit 256]
        S4[--memory 1GB]
        S5[--cpus 2]
        S6[--network host]
        S7[--rm auto-cleanup]
        S8[Bind mount validation<br/>blocks .ssh .aws .gnupg .env]
    end
```

Chat agents bypass containers entirely — they run as direct `claude` subprocesses for lower latency.

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `TELEGRAM_BOT_TOKEN` | (required) | Telegram Bot API token |
| `ASSISTANT_NAME` | `Borg` | Bot display name and trigger word |
| `CLAUDE_MODEL` | `claude-sonnet-4-6` | Model for Claude Code CLI |
| `COLLECTION_WINDOW_MS` | `3000` | Message batching window |
| `COOLDOWN_MS` | `5000` | Post-agent cooldown |
| `AGENT_TIMEOUT_S` | `600` | Max agent runtime |
| `MAX_CONCURRENT_AGENTS` | `4` | Global agent thread limit |
| `RATE_LIMIT_PER_MINUTE` | `5` | Triggers per minute per group |
| `PIPELINE_REPO` | (empty) | Target repo path (enables pipeline) |
| `PIPELINE_TEST_CMD` | `zig build test` | Test command for pipeline |
| `RELEASE_INTERVAL_MINS` | `180` | Minutes between release trains |
| `PIPELINE_ADMIN_CHAT` | (empty) | Chat ID for pipeline notifications |
| `WHATSAPP_ENABLED` | `false` | Enable WhatsApp bridge |
