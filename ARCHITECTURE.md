# Borg Legal AI Agent — Architecture Specification
## Agreed Implementation Spec (v1)

---

## 1. Infrastructure Layer

### LLM Inference
- **Provider**: AWS Bedrock (Claude models)
- **Why**: Structural data isolation — Anthropic cannot see inputs/outputs. AWS BAA is self-serve via AWS Artifact (no sales process). GDPR DPA auto-applies. ZDR-equivalent by architecture.
- **Not**: Anthropic direct API (requires sales negotiation for ZDR), Google Vertex AI (BAA requires sales contact)
- **Region-locking**: Enforce per firm. EU firms → `eu-central-1` (Frankfurt) or `eu-west-1`. UK firms → `eu-west-2` (London). US firms → `us-east-1` / `us-west-2`.

### Web Search
- **Provider**: Brave Search Enterprise API with ZDR enabled
- **Why**: Only search provider with a fully independent index (not scraping Google/Bing), meaning ZDR applies to every query end-to-end. SOC 2 Type II certified (2025). DPA available.
- **ZDR activation**: Contact Brave API support (searchapi-support@brave.com) to enable enterprise ZDR plan. Self-serve signup available for development; ZDR upgrade required before production.
- **Not**: Anthropic's native web_search tool (routes through third-party provider not covered by Borg's ZDR agreements)

### Key Management
- **Requirement**: All secrets and master encryption keys must be managed via AWS KMS (Customer Managed Keys)
- **Not permitted**: Environment variable injection of master keys (visible to all processes under same user, leaked in crash dumps, not auditable, not rotatable without restart)
- **Implementation**: Borg Server retrieves keys from KMS at runtime via IAM role-scoped credentials. Keys never exist in plaintext in the application environment. (Fallback to `BORG_MASTER_KEY` env var allowed only for local dev/non-AWS deployments).

---

## 2. Container Architecture

### Drafter Agent (Air-Gapped)
- Runs in a container with **no network interface** (`--unshare-net`)
- Physically cannot open TCP/UDP sockets to the open internet
- Communicates exclusively with the Borg Server via Unix socket or `127.0.0.1:3132`
- Holds all confidential client documents in memory/local workspace
- Has no direct access to Bedrock, Brave, or any external API

### Borg Server (Proxy Layer)
- Acts as the sole network-capable component
- Intercepts all LLM API calls from the Drafter Agent and forwards to Bedrock
- Intercepts all `web_search` tool calls and routes through the search pipeline (see Section 3)
- Manages KMS key retrieval and envelope encryption
- Owns the audit logging pipeline
- Signs all outbound requests

---

## 3. Privilege Architecture: The Phase Split

Borg implements a strict **One-Way Phase Gate** to guarantee that privileged data is never exposed to a search engine, even a compliant one.

### Phase 1: Research (Unprivileged)
*   **State:** `session_privileged = false`
*   **Capabilities:** Full Brave Search ZDR access.
*   **Restrictions:** **NO privileged document upload permitted.** The UI physically hides the upload mechanism for sensitive files.
*   **Goal:** The agent performs open-ended legal research, precedent finding, and market analysis *before* seeing the confidential client facts.

### Phase 2: Execution (Privileged)
*   **Trigger:** The moment the user uploads the first privileged document.
*   **State:** `session_privileged = true`. This transition is **permanent** for the session.
*   **Restrictions:** **Search is Hard-Blocked.** The Borg Server rejects all `web_search` tool calls.
*   **Capabilities:** Only Bedrock Inference (via PrivateLink) is allowed.
*   **Goal:** The agent uses the research from Phase 1 and the privileged documents from Phase 2 to draft the final work product.

### Routing Logic

| Session State | Bedrock Inference | Brave Search |
|---|---|---|
| Phase 1 (Research) | ✅ Allowed | ✅ Allowed (ZDR) |
| Phase 2 (Execution) | ✅ Allowed | ❌ **Hard Disabled** |

---

## 4. Search Pipeline (Phase 1 Only)

### Non-Privileged Sessions
- Drafter Agent issues `web_search` tool call with freeform query
- Borg Server forwards directly to Brave Search Enterprise API
- ZDR ensures no retention by Brave

*(Note: The previously proposed "Sanitisation Pipeline" for privileged sessions has been removed. Privileged sessions simply cannot search. This eliminates the risk of "leaky sanitisation" entirely.)*

---

## 5. Lifecycle Management

### Burn-After-Reading
- Configurable retention window **per matter type** (not a hardcoded 7-day idle trigger)
- Default: prompt the firm to configure at onboarding per their SRA file retention obligations
  - Standard matters: 6 years post-matter-close (UK SRA default)
  - Wills/property: configurable to indefinite
- On retention expiry: destroy all vector embeddings, chat logs, document content, and disk workspaces
- **Audit metadata is retained** (who, when, which API, which tool — but never the payload content)

### Audit Trail
- Immutable log of: user identity, timestamp, API endpoint hit, tool called, session ID
- **Phase Transitions:** Specifically logs the timestamp and user who triggered the transition from Phase 1 to Phase 2.
- No sensitive payload content in logs
- Stored in your AWS account (CloudTrail + CloudWatch), never in Borg's infrastructure
- Retained per firm's own compliance requirements (malpractice defence window)

---

## 6. Encryption

### At Rest
- AES-256-GCM for all stored data (SQLite or equivalent)
- Master key via AWS KMS CMK (not environment variable — see Section 1)

### In Transit
- All communication between Drafter Agent and Borg Server: Unix socket (no network)
- All communication between Borg Server and Bedrock/Brave: TLS 1.2+ enforced
- VPC + AWS PrivateLink for Bedrock calls (no public internet egress from within the VPC)

---

## 7. Contractual / Legal Structure (Non-Engineering, Required Before Production)

Borg signs all sub-processor agreements. Law firms sign one agreement with Borg only.

### Borg signs with sub-processors:
- **AWS**: BAA (self-serve via AWS Artifact) + GDPR DPA (auto-applies)
- **Brave Search**: Enterprise ZDR DPA (contact searchapi-support@brave.com)

### Law firms sign with Borg:
- **Borg DPA** (Article 28 GDPR/UK GDPR compliant): names AWS and Brave as sub-processors, documents what each sub-processor does, gives firms approval rights over changes to sub-processor list
- **BAA passthrough** (if US HIPAA-regulated work): Borg's BAA with AWS covers this via the subcontractor chain

### Required before onboarding any firm:
- DPIA completed and documented (UK GDPR Article 35 / EU GDPR mandatory for AI processing personal data)
- Borg DPA executed with firm

---

## 8. Human-in-the-Loop

- Borg produces documents only — it does not send, file, or take any automated external action
- All outputs are reviewed by a solicitor before use
- This satisfies the *Heppner* doctrine requirement that AI be used "under attorney direction"
- No additional enforcement gate required in V1 (the product design itself enforces this)

---

## 9. Privilege Preservation Conditions (Post-Heppner)

For privileged sessions, all three must be met for privilege to be preserved:

1. **Contractual confidentiality**: Borg's DPA with the firm + AWS BAA chain ✅
2. **Solicitor-operated**: Borg is operated by the solicitor, not the client directly — enforce via auth/access controls
3. **Reasonable expectation of confidentiality**: Bedrock BAA eliminates any right to retain, train, or disclose — unlike consumer AI tools which waived privilege in *Heppner*

---

## 10. V1 Engineering Checklist

| Item | Status |
|---|---|
| Air-gapped Drafter Agent container (`--unshare-net`) | ⬜ Build |
| Borg Server inference proxy → Bedrock | ⬜ Build |
| Borg Server search proxy → Brave | ⬜ Build |
| `privileged` bool on document ingestion | ⬜ Build |
| Session contamination flag | ⬜ Build |
| Phase 1 / Phase 2 hard boundary — session state enforcement | ⬜ Build |
| Phase 1 UI — no privileged document upload mechanism present | ⬜ Build |
| Phase 2 — Borg Server hard-blocks all search tool calls | ⬜ Build |
| Phase 1 cannot reopen after Phase 2 entered | ⬜ Build |
| Audit log entry on phase transition (timestamp, user, matter ID) | ⬜ Build |
| Burn-after-reading lifecycle (configurable per matter type) | ⬜ Build |
| Audit metadata trail (no payload content) | ⬜ Build |
| AWS KMS CMK for master key management | ⬜ Build |
| AES-256-GCM encryption at rest | ✅ Done |
| VPC + PrivateLink for Bedrock | ⬜ Build |
| AWS BAA (self-serve, AWS Artifact) | ⬜ Ops |
| Brave Enterprise ZDR (email support team) | ⬜ Ops |
| Borg DPA template | ⬜ Legal |
| DPIA template | ⬜ Legal |
| SOC 2 Type II readiness | ⬜ Future |
| ISO 27001 | ⬜ Future |
| Pentest programme | ⬜ Future |
