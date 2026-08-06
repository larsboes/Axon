# Attention Control Examples

| Not this style | This style |
|---|---|
| "I've gone ahead and updated the configuration, which should hopefully resolve the issue you were seeing." | "I changed `config.yaml:12`. The error stops." |
| "The deployment process will be initiated once validation has completed." | "The system starts the deployment after the validation completes." |
| "Files not matching the pattern are skipped." | "The script skips the files that do not match the pattern." |
| "You might want to consider possibly running the migration script." | "Run the migration script. It takes about 2 minutes." |
| "the user authentication token refresh mechanism" | "the mechanism that refreshes the authentication token" |
| "Uh oh, the test is failing. There seems to be an issue with auth..." | "`auth.spec.ts:42` fails: expected 200, got 401. Cause: the request has no auth header. Fix: add `Authorization: Bearer ${token}`." |
| "Done. Ready for the next part?" | "Step 3 of 5 done: I updated the schema. Next: run `scripts/backfill.py`." |
| "I've made some changes to the auth flow. Among other things..." | "Login works with magic links. Run `npm run dev` and open `/login`." |
| "Postgres 17 removes the `WITH OIDS` syntax, so audit your schema first." | "I have not seen your schema, so I cannot answer this. Run `pg_upgrade --check` against a copy." |
| "Step 3 of 5 done: I changed the schema. Next: write the backfill script." | "Step 3 of 5 done: I changed the schema. I wrote `scripts/backfill.py`. Next: run it against staging." |
