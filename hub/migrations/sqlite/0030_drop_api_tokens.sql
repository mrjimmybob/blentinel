-- Pre-RC audit F5 (D-119): drop 0027's `api_tokens`.
--
-- 0027 came from abandoned M1-07 work: no code or test ever used the table,
-- and M1-07 (deferred until after the first RC) will design its own. 0027
-- itself stays, because a committed migration is never edited (D-111).

DROP INDEX idx_api_tokens_prefix;
DROP INDEX idx_api_tokens_user_id;
DROP TABLE api_tokens;
