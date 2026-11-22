# Logging improvements

Checklist derived from the logging-system review. Executed in order.

- [x] 1. Introduce a file-backed application logger (`log` + `time`), writing
      `<app_data>/logs/easyai.log` with timestamps and levels.
- [x] 2. Enrich job logs: timestamps per line, method/origin in the header,
      exit code and duration for every command, total job duration.
- [x] 3. Add rotation and retention: rotate the application and service logs by
      size, prune old job rows/logs on startup, delete per-app logs on removal.
- [x] 4. Surface background failures: log state-refresh and update-check errors;
      add a `log_ui_error` command and report previously swallowed UI errors.
- [x] 5. Redact secrets in log output (credential assignments, sensitive flags,
      signed URL parameters, authorization headers).
- [x] 6. Unify the UI log view: one stream for live and historical job logs,
      bounded in memory.

## Decisions

- Application log: `easyai.log`, rotated at 5 MiB to `easyai.log.1`. Level is
  `info` by default and can be overridden with `EASYAI_LOG`.
- Service logs (`<app_id>-service.log`) rotate at 32 MiB to `.1` on start.
- Job output and records are capped at the newest 200 across all applications;
  older rows and their files are removed at startup.
- Uninstalling keeps the job history (audit trail). Removing an application from
  the catalog deletes its jobs, job logs, and service log.
- Job log lines are `[YYYY-MM-DD HH:MM:SS] <text>`, UTC.
- Redaction applies to job-log lines, the application log, and the stored/emitted
  job error. The command's return value to the caller is unchanged.
