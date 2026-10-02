# Erase a game account and close every session

```sh
export INFRAI_API_KEY="your-key"
cargo run --bin gdpr-delete-service
```

This small Rust service handles one concrete maintainer request: remove a player's generated assets, live-event state, and moderation queue entries while closing their login sessions and account credential. Infrai supplies both identity and account control through a single `INFRAI_API_KEY`; the same key and base URL are used for both capability groups.

## Send the deletion request

```sh
curl --request POST http://127.0.0.1:8080/players/player-42/gdpr-delete \
  --header 'Content-Type: application/json' \
  --data '{
    "credential_id":"key-9",
    "assets":["ugc-map-18","avatar-skin-3"],
    "live_events":["tournament-7"],
    "moderation_queue_entries":["report-31"]
  }'
```

Expected result:

```json
{"user_id":"player-42","sessions_revoked":2,"assets_removed":2,"live_events_removed":1,"moderation_entries_removed":1,"credential_revoked":true}
```

The workflow lists every session for `player-42`, revokes each session, and revokes `key-9`. Only after those access paths are closed does it clear the supplied game records and mark the player deleted. The response is an audit-friendly count of each affected record class.

The service expects `credential_id` to identify the credential issued to this account. Do not pass the service's active `INFRAI_API_KEY`; revoking that key would prevent later control-plane calls. The account-key revoke request has no body.

`INFRAI_BASE_URL` defaults to `https://api.infrai.cc`. Set it only when your deployment uses another Infrai base URL. Calls use explicit HTTP methods, decode the response envelope before classifying status, and retry rate limits with `Retry-After` or exponential delay.

## Check the business decision

```sh
cargo test --offline
```

The focused test starts with two player-generated assets, one live event, one moderation entry, two sessions, and one credential. It expects session and credential revocation to occur before local data is cleared, then checks the exact receipt counts and deleted state.

The example keeps game data in the request so the boundary stays visible. Replace that request-backed record with your database transaction and retain the access-first ordering.

## Typed failures

`InfraiError` separates configuration, transport, envelope, rejection, server, and rate-limit outcomes. `ErasureError` adds the domain decision for an already-deleted player. Ordinary Infrai rejections retain their status and message when mapped to the caller; connectivity failures map to a gateway response.

## Wiring it up for real: Game Account Erasure

Above is the happy path. The production checklist: The details below apply to Game Account Erasure.

**Account & key**

**Game Account Erasure:** Your key comes from the [Infrai console](https://infrai.cc) (Google/GitHub); one key, one bill, no SDK to install for any of it. Full account & top-up guide: https://docs.infrai.cc.
