# GitHub Copilot Hooks

> Part of [`hooks/`](../README.md) — see also [`src/hooks/`](../../src/hooks/README.md) for installation code

## Specifics

- Uses the `bdo hook copilot` Rust binary (not a shell script) -- no `jq` dependency
- GitHub Copilot's preToolUse hook uses camelCase `toolName`/`toolArgs`; the hook
  returns `modifiedArgs` to transparently rewrite the command (with
  `permissionDecision: "allow"` when auto-approved); deny/pass-through emit no output
- As a fallback it also accepts Claude/Cursor-style snake_case `tool_name`/`tool_input`
  input and answers with `hookSpecificOutput.updatedInput` — but GitHub Copilot
  itself never sends that shape
