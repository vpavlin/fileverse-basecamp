#!/usr/bin/env bash
# End-to-end run of the portal program against a local LEZ v0.4.0-rc3 chain, with `spel`.
#
# NOT YET RUN AGAINST A CHAIN by the author (no v0.4 chain on the machine that wrote it);
# the program's logic is covered by `cargo test` (in-process conversations). Expect to fix
# flag details on first run, and please send the diffs back.
#
# Needs:
#   * `spel` built from logos-co/spel branch refactor/lez-v0.4.x (rev d7444f84 or newer);
#   * a running v0.4.0-rc3 sequencer and a wallet in $LEE_WALLET_HOME_DIR whose config
#     points at it, holding three PUBLIC accounts labelled alice, bob and payer, payer funded;
#     ALICE_ID, BOB_ID: their base58 ids (labels do not resolve inside JSON arguments);
#   * target/guest/portal.bin (scripts/build-guest.sh) or PORTAL_BIN=<path to the .bin>;
#   * jq, and a Rust toolchain (the address helper is `cargo run -p portal_client --example addr`).
#
# Flow: deploy -> create portal -> add collaborator -> register keys -> add / edit / delete
# files -> list files -> registry lookups, asserting what each step changed.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$here"
: "${ALICE_ID:?base58 id of the public account labelled alice}"
: "${BOB_ID:?base58 id of the public account labelled bob}"
: "${LEE_WALLET_HOME_DIR:?wallet home}"
bin="${PORTAL_BIN:-target/guest/portal.bin}"
IDL=artifacts/portal-idl.json
payer=payer

say() { printf '\n== %s\n' "$*"; }
die() { echo "FAIL: $*" >&2; exit 1; }

say "deploy"
PROGRAM="$(spel deploy "$bin" --fee-payer "$payer")"
echo "program: $PROGRAM"

addr() { cargo run -q -p portal_client --example addr -- "$PROGRAM" "$@"; }
spel_send() { spel --idl "$IDL" --program "$PROGRAM" --fee-payer "$payer" send "$@"; }
inspect() { spel --idl "$IDL" --program "$PROGRAM" inspect "$1@$PROGRAM"; }
# Run a send, require it to change state (exit 0).
send() { spel_send "$@" >/dev/null || die "send failed: $*"; }

say "create a portal (alice owns it)"
REGISTRY=$(addr registry); OWNER_HEAD=$(addr owner "$ALICE_ID")
# Empty actors print null; counters start at 0.
owner_count=$(inspect "$OWNER_HEAD" 2>/dev/null | jq -r '.OwnerHead.count // 0')
registry_count=$(inspect "$REGISTRY" 2>/dev/null | jq -r '.RegistryHead.portal_count // 0')
PORTAL=$(addr portal "$ALICE_ID" "$owner_count")
send --root alice --signer alice \
  --actor "$OWNER_HEAD" --actor "$PORTAL" \
  --actor "$(addr owned "$ALICE_ID" "$owner_count")" \
  --actor "$REGISTRY" --actor "$(addr all "$registry_count")" \
  -- create-portal --owner-did did:alice --metadata-cid bafymeta
[ "$(inspect "$PORTAL" | jq -r '.Portal.owner')" = "$ALICE_ID" ] || die "portal owner"
echo "portal: $PORTAL"

say "registry lookups"
[ "$(inspect "$(addr all "$registry_count")" | jq -r '.AllEntry.portal')" = "$PORTAL" ] || die "all entry"
[ "$(inspect "$(addr owned "$ALICE_ID" "$owner_count")" | jq -r '.OwnerEntry.portal')" = "$PORTAL" ] || die "owner entry"

say "add collaborator bob, register keys"
send --root alice --signer alice --actor "$PORTAL" -- act --portal "$PORTAL" \
  --op "{\"AddCollaborator\":{\"who\":\"$BOB_ID\"}}"
send --root bob --signer bob --actor "$PORTAL" -- act --portal "$PORTAL" \
  --op '{"RegisterKeys":{"view_did":"did:view:bob","edit_did":"did:edit:bob"}}'
[ "$(inspect "$PORTAL" | jq -r '.Portal.collaborators | length')" = 1 ] || die "collaborators"
[ "$(inspect "$PORTAL" | jq -r '.Portal.collaborator_keys | length')" = 1 ] || die "keys"

say "add two files (alice, bob)"
file_count=$(inspect "$PORTAL" | jq -r '.Portal.file_count')
add_file() { # caller app_file_id cid
  local fc; fc=$(inspect "$PORTAL" | jq -r '.Portal.file_count')
  send --root "$1" --signer "$1" --actor "$PORTAL" \
    --actor "$(addr file "$PORTAL" "$fc")" --actor "$(addr appfile "$PORTAL" "$2")" \
    -- act --portal "$PORTAL" \
    --op "{\"AddFile\":{\"app_file_id\":\"$2\",\"file_type\":\"Public\",\"metadata_cid\":\"$3-meta\",\"content_cid\":\"$3-content\",\"gate_cid\":\"\",\"version\":1}}"
}
add_file alice doc-a bafya
add_file bob doc-b bafyb
[ "$(inspect "$PORTAL" | jq -r '.Portal.file_count')" = 2 ] || die "file_count"

say "edit file 0 (bob), delete file 1 by index (alice)"
send --root bob --signer bob --actor "$PORTAL" --actor "$(addr file "$PORTAL" 0)" \
  -- act --portal "$PORTAL" \
  --op '{"EditFile":{"index":0,"file_type":"Gated","metadata_cid":"m2","content_cid":"c2","gate_cid":"g2","version":2}}'
[ "$(inspect "$(addr file "$PORTAL" 0)" | jq -r '.File.content_cid')" = c2 ] || die "edit"
send --root alice --signer alice --actor "$PORTAL" --actor "$(addr file "$PORTAL" 1)" \
  -- act --portal "$PORTAL" --op '{"DeleteFile":{"index":1}}'
[ "$(inspect "$(addr file "$PORTAL" 1)" | jq -r '.File.deleted')" = true ] || die "delete"

say "list files"
count=$(inspect "$PORTAL" | jq -r '.Portal.file_count')
for ((i = 0; i < count; i++)); do
  inspect "$(addr file "$PORTAL" "$i")" | jq -c '.File | {index, app_file_id, deleted, content_cid}'
done

say "delete by app file id"
add_file alice doc-c bafyc
idx=$(inspect "$(addr appfile "$PORTAL" doc-c)" | jq -r '.AppFile.index')
send --root alice --signer alice --actor "$PORTAL" \
  --actor "$(addr appfile "$PORTAL" doc-c)" --actor "$(addr file "$PORTAL" "$idx")" \
  -- act --portal "$PORTAL" --op '{"DeleteFileByAppFileId":{"app_file_id":"doc-c"}}'
[ "$(inspect "$(addr file "$PORTAL" "$idx")" | jq -r '.File.deleted')" = true ] || die "delete by app id"

say "ownership transfer to bob"
send --root alice --signer alice --actor "$PORTAL" -- act --portal "$PORTAL" \
  --op "{\"TransferOwnership\":{\"new_owner\":\"$BOB_ID\"}}"
bob_count=$(inspect "$(addr owner "$BOB_ID")" 2>/dev/null | jq -r '.OwnerHead.count // 0')
send --root bob --signer bob --actor "$PORTAL" --actor "$(addr owner "$BOB_ID")" \
  --actor "$(addr owned "$BOB_ID" "$bob_count")" \
  -- act --portal "$PORTAL" --op '"AcceptOwnership"'
[ "$(inspect "$PORTAL" | jq -r '.Portal.owner')" = "$BOB_ID" ] || die "ownership"

say "OK"
