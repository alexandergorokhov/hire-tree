# receipt

## Purpose
Prove on-chain that a deliverable came out of a NEAR AI Cloud model call made
for this exact task and this exact delivery, without trusting the worker.

NEAR AI Cloud signs `"{model}:{sha256_hex(request)}:{sha256_hex(response)}"`
as an EIP-191 personal message with a secp256k1 key held in its TEE, and
publishes it at `GET /v1/signature/{chat_id}?model=..&signing_algo=ecdsa`.

## Requirements

### Requirement: Signature check
`deliver(resp_hash, req, sig, model)` with a receipt SHALL rebuild the text
from `model`, `sha256(req)` (computed by the contract) and `resp_hash`, take
the EIP-191 digest, `env::ecrecover` it with the malleability check on, and
compare `keccak256(pubkey)[12..]` with the job's pinned signer. All of `req`,
`sig`, `model` are required together.

#### Scenario: Valid receipt
- WHEN the worker delivers with a receipt signed by the pinned key over the bound request and the claimed response hash
- THEN the delivery is stored with `verified = true`
- unit: `valid_receipt_marks_delivery_verified`

#### Scenario: Real NEAR AI Cloud receipt text
- WHEN core rebuilds the text of a receipt recorded on testnet (Agent Exit G6, public data)
- THEN it equals the text NEAR AI Cloud returned
- core: `receipt::tests::recorded_text_matches_near_ai_cloud`

#### Scenario: Wrong signer
- WHEN the signature recovers to another address
- THEN deliver panics "receipt: signer is not the pinned NEAR AI Cloud key"
- unit: `receipt_from_other_key_rejected`

#### Scenario: Response swapped
- WHEN `resp_hash` differs from the hash that was signed
- THEN recovery yields another address and deliver panics
- unit: `receipt_with_swapped_response_rejected`

#### Scenario: Malformed or malleable signature
- WHEN `sig` is not 65 bytes hex, `v` is not 0/1/27/28, or `s` is high
- THEN deliver panics and nothing changes
- unit: `malformed_signature_rejected`, `high_s_signature_rejected`

#### Scenario: Partial receipt
- WHEN only some of `req`, `sig`, `model` are given
- THEN deliver panics "receipt needs req, sig and model together"
- unit: `partial_receipt_rejected`

### Requirement: Binding to task and delivery
The request bytes SHALL contain exactly one tag `HT1:<this account>:<spec_hash hex>:<seq>`
where seq is this delivery's number (previous seq + 1); `req` is at most 8 KB;
`model` is 1..=128 bytes.

#### Scenario: Copied receipt from another task
- WHEN a worker submits a valid receipt whose tag names another job account
- THEN deliver panics "binding tag ... is not this task"
- unit: `receipt_copied_from_other_task_rejected`

#### Scenario: Request without tag
- unit: `receipt_without_binding_tag_rejected`

#### Scenario: Replayed receipt on a redelivery
- WHEN the worker redelivers with the receipt of its own earlier delivery (old seq)
- THEN deliver panics (tag seq mismatch)
- unit: `receipt_replayed_on_redelivery_rejected`

#### Scenario: Two tags in one request
- unit: core `binding::tests` (MultipleTags)

#### Scenario: Oversized request or model
- unit: `oversized_request_rejected`, `oversized_model_rejected`

### Requirement: Delivery without a receipt
A worker MAY deliver with `resp_hash` only. It is stored with
`verified = false`; it is paid at settle but its note cannot be sold.
- unit: `worker_delivers`, `quote_note_is_none_without_verified_receipt`
