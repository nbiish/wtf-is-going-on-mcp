# TASK no-local-fcc-spawn (2026-09-07)

- Operator: wtf must never spawn local-router or fcc-server itself; infra is operator-installed and already running.
- Remove ensure/restart/supervisor paths in executor, main(serve), api, mcp, dashboard; keep passive :11434 probe.
- Executor env injection for agent CLIs becomes conditional on router being up; otherwise harnesses use their own config.
