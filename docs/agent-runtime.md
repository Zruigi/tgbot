# Agent Runtime Architecture

## Request Path

The MiniApp does not call an n8n Agent runtime. Its production path is:

1. MiniApp sends a request to `POST /api/ai/chat`.
2. The Python structured intent router selects a registered tool.
3. Read tools execute before any user-facing answer is generated.
4. Destructive tools create an exact, expiring snapshot and wait for confirmation.
5. The Python backend revalidates tenant scope, message identity, sender role, Bot
   permissions, and feature switches before calling the Telegram Bot SDK.
6. The result and all partial failures are written to the audit log and returned to
   the same Agent conversation.

n8n is an optional workflow adapter. It discovers the same tool contract through
`GET /api/agent/tools/manifest` and calls Python HTTP endpoints. It must not pull
Telegram `getUpdates`, own a second Bot instance, or bypass Python safety checks.

## Advertisement Decision Path

Messages enter through the Python Telegram Bot and are saved once in SQLite. The
advertisement pipeline evaluates them in this order:

1. Normalize text and check the tenant-scoped confirmed text fingerprint library.
2. Apply deterministic advertising signals.
3. Group exact duplicates without sending each message to an LLM.
4. Use pgvector/Ollama clustering for unknown text when the optional PostgreSQL
   pipeline is enabled.
5. Send only unresolved cluster summaries to an LLM for structured classification.

Successfully deleted advertisements seed the fingerprint library. A later exact
normalized match is marked with high confidence at ingestion time and consumes no
LLM tokens.

## Deletion Policy

- Natural-language requests such as `删除最近2条广告` create a bounded snapshot.
- A batch is capped at 50 records and expires after 15 minutes.
- Groups without `can_delete_messages` are excluded before confirmation.
- Admin, Bot, channel-forwarded, deleted, and low-score records are excluded.
- Sender-history expansion happens only when the user explicitly requests it, and
  only for senders with a confirmed advertisement in a deletable group.
- `AD_DELETE_ENABLED` controls message deletion. `AD_KICK_ENABLED` remains a
  separate, disabled-by-default permission.
