# codeless skills

This directory holds `ai-ui` skill files: one `.md` per skill, with YAML
frontmatter describing the skill (name, description, optional component
allowlist). At request time the codeless-server's `/api/ai-ui/chat` route
concatenates the relevant skills' bodies into the system prompt sent to
the AI provider.

Skills are user-extensible — drop a new `.md` here and restart the
server. No code change required.

See [ai-ui SCOPE.md](../../ai-ui/SCOPE.md) for the skill file format and
[skills/](../../ai-ui/skills/) for working examples (e.g.
`iot-dashboard.md`, `scope-preview.md`).
