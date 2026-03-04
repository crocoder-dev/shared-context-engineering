use anyhow::{bail, ensure, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostedProvider {
    GitHub,
    GitLab,
}

impl HostedProvider {
    fn as_str(&self) -> &'static str {
        match self {
            HostedProvider::GitHub => "github",
            HostedProvider::GitLab => "gitlab",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostedWebhookRequest {
    pub provider: HostedProvider,
    pub event: String,
    pub signature: String,
    pub delivery_id: Option<String>,
    pub shared_secret: String,
    pub payload_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostedReconciliationRunRequest {
    pub provider: HostedProvider,
    pub repository: String,
    pub event: String,
    pub old_head: String,
    pub new_head: String,
    pub idempotency_key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconciliationRunInsertOutcome {
    Created,
    Duplicate,
}

pub trait ReconciliationRunStore {
    fn insert_run(
        &mut self,
        request: HostedReconciliationRunRequest,
    ) -> Result<ReconciliationRunInsertOutcome>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostedIntakeOutcome {
    Created(HostedReconciliationRunRequest),
    Duplicate(HostedReconciliationRunRequest),
}

pub fn ingest_hosted_rewrite_event(
    request: HostedWebhookRequest,
    run_store: &mut impl ReconciliationRunStore,
) -> Result<HostedIntakeOutcome> {
    verify_signature(&request)?;
    let run_request = parse_run_request(&request)?;

    let outcome = run_store.insert_run(run_request.clone())?;
    match outcome {
        ReconciliationRunInsertOutcome::Created => Ok(HostedIntakeOutcome::Created(run_request)),
        ReconciliationRunInsertOutcome::Duplicate => {
            Ok(HostedIntakeOutcome::Duplicate(run_request))
        }
    }
}

fn parse_run_request(request: &HostedWebhookRequest) -> Result<HostedReconciliationRunRequest> {
    let old_head = find_required_json_string(&request.payload_json, "before")?;
    let new_head = find_required_json_string(&request.payload_json, "after")?;

    ensure!(
        is_sha_like(&old_head),
        "invalid hosted event payload: 'before' is not a git SHA"
    );
    ensure!(
        is_sha_like(&new_head),
        "invalid hosted event payload: 'after' is not a git SHA"
    );

    let repository = match request.provider {
        HostedProvider::GitHub => find_required_json_string(&request.payload_json, "full_name")?,
        HostedProvider::GitLab => {
            find_required_json_string(&request.payload_json, "path_with_namespace")?
        }
    };

    let idempotency_key = derive_idempotency_key(
        request.provider,
        &request.event,
        &repository,
        &old_head,
        &new_head,
        request.delivery_id.as_deref(),
    );

    Ok(HostedReconciliationRunRequest {
        provider: request.provider,
        repository,
        event: request.event.clone(),
        old_head,
        new_head,
        idempotency_key,
    })
}

fn verify_signature(request: &HostedWebhookRequest) -> Result<()> {
    ensure!(
        !request.signature.trim().is_empty(),
        "missing hosted event signature"
    );

    match request.provider {
        HostedProvider::GitHub => {
            let expected = github_signature(&request.shared_secret, &request.payload_json);
            ensure!(
                constant_time_eq(request.signature.as_bytes(), expected.as_bytes()),
                "hosted event signature verification failed for github"
            );
        }
        HostedProvider::GitLab => {
            ensure!(
                constant_time_eq(
                    request.signature.as_bytes(),
                    request.shared_secret.as_bytes()
                ),
                "hosted event signature verification failed for gitlab"
            );
        }
    }

    Ok(())
}

fn derive_idempotency_key(
    provider: HostedProvider,
    event: &str,
    repository: &str,
    old_head: &str,
    new_head: &str,
    delivery_id: Option<&str>,
) -> String {
    let delivery = delivery_id.unwrap_or("no-delivery-id");
    let material = format!(
        "provider={};event={};repo={};before={};after={};delivery={}",
        provider.as_str(),
        event,
        repository,
        old_head,
        new_head,
        delivery
    );
    let digest = hex_lower(&sha256(material.as_bytes()));
    format!("hosted:{}:{}", provider.as_str(), digest)
}

fn find_required_json_string(payload: &str, key: &str) -> Result<String> {
    let key_pattern = format!("\"{}\"", key);
    let Some(key_start) = payload.find(&key_pattern) else {
        bail!("invalid hosted event payload: missing '{}' field", key);
    };

    let mut idx = key_start + key_pattern.len();
    while idx < payload.len() && payload.as_bytes()[idx].is_ascii_whitespace() {
        idx += 1;
    }

    ensure!(
        idx < payload.len() && payload.as_bytes()[idx] == b':',
        "invalid hosted event payload: malformed '{}' field",
        key
    );
    idx += 1;

    while idx < payload.len() && payload.as_bytes()[idx].is_ascii_whitespace() {
        idx += 1;
    }

    ensure!(
        idx < payload.len() && payload.as_bytes()[idx] == b'"',
        "invalid hosted event payload: '{}' field must be a string",
        key
    );
    idx += 1;

    let mut value = String::new();
    let mut escaped = false;
    while idx < payload.len() {
        let byte = payload.as_bytes()[idx];
        idx += 1;
        if escaped {
            value.push(byte as char);
            escaped = false;
            continue;
        }

        if byte == b'\\' {
            escaped = true;
            continue;
        }

        if byte == b'"' {
            return Ok(value);
        }

        value.push(byte as char);
    }

    bail!(
        "invalid hosted event payload: unterminated '{}' string",
        key
    )
}

fn is_sha_like(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn github_signature(secret: &str, payload: &str) -> String {
    let mac = hmac_sha256(secret.as_bytes(), payload.as_bytes());
    format!("sha256={}", hex_lower(&mac))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut diff: u8 = 0;
    for (lhs, rhs) in left.iter().zip(right.iter()) {
        diff |= lhs ^ rhs;
    }

    diff == 0
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0_u8; BLOCK_SIZE];

    if key.len() > BLOCK_SIZE {
        let hashed = sha256(key);
        key_block[..hashed.len()].copy_from_slice(&hashed);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0_u8; BLOCK_SIZE];
    let mut outer_pad = [0_u8; BLOCK_SIZE];
    for idx in 0..BLOCK_SIZE {
        inner_pad[idx] = key_block[idx] ^ 0x36;
        outer_pad[idx] = key_block[idx] ^ 0x5c;
    }

    let mut inner_input = Vec::with_capacity(BLOCK_SIZE + message.len());
    inner_input.extend_from_slice(&inner_pad);
    inner_input.extend_from_slice(message);
    let inner_hash = sha256(&inner_input);

    let mut outer_input = Vec::with_capacity(BLOCK_SIZE + inner_hash.len());
    outer_input.extend_from_slice(&outer_pad);
    outer_input.extend_from_slice(&inner_hash);

    sha256(&outer_input)
}

fn sha256(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut padded = input.to_vec();
    let bit_len = (padded.len() as u64) * 8;
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut message_schedule = [0_u32; 64];
    for chunk in padded.chunks_exact(64) {
        for (idx, word) in chunk.chunks_exact(4).take(16).enumerate() {
            message_schedule[idx] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }

        for idx in 16..64 {
            let s0 = message_schedule[idx - 15].rotate_right(7)
                ^ message_schedule[idx - 15].rotate_right(18)
                ^ (message_schedule[idx - 15] >> 3);
            let s1 = message_schedule[idx - 2].rotate_right(17)
                ^ message_schedule[idx - 2].rotate_right(19)
                ^ (message_schedule[idx - 2] >> 10);
            message_schedule[idx] = message_schedule[idx - 16]
                .wrapping_add(s0)
                .wrapping_add(message_schedule[idx - 7])
                .wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];

        for idx in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[idx])
                .wrapping_add(message_schedule[idx]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut output = [0_u8; 32];
    for (idx, value) in h.iter().enumerate() {
        output[idx * 4..idx * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }
    output
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use anyhow::Result;

    use super::{
        derive_idempotency_key, github_signature, ingest_hosted_rewrite_event, HostedIntakeOutcome,
        HostedProvider, HostedReconciliationRunRequest, HostedWebhookRequest,
        ReconciliationRunInsertOutcome, ReconciliationRunStore,
    };

    #[derive(Default)]
    struct FakeReconciliationRunStore {
        inserted: Vec<HostedReconciliationRunRequest>,
        seen_keys: HashSet<String>,
    }

    impl ReconciliationRunStore for FakeReconciliationRunStore {
        fn insert_run(
            &mut self,
            request: HostedReconciliationRunRequest,
        ) -> Result<ReconciliationRunInsertOutcome> {
            self.inserted.push(request.clone());
            let inserted = self.seen_keys.insert(request.idempotency_key);
            Ok(if inserted {
                ReconciliationRunInsertOutcome::Created
            } else {
                ReconciliationRunInsertOutcome::Duplicate
            })
        }
    }

    fn github_payload() -> String {
        "{\"before\":\"1111111111111111111111111111111111111111\",\"after\":\"2222222222222222222222222222222222222222\",\"repository\":{\"full_name\":\"acme/sce\"}}".to_string()
    }

    fn gitlab_payload() -> String {
        "{\"before\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"after\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"project\":{\"path_with_namespace\":\"group/sce\"}}".to_string()
    }

    #[test]
    fn github_intake_verifies_signature_and_creates_run() -> Result<()> {
        let payload = github_payload();
        let mut store = FakeReconciliationRunStore::default();
        let request = HostedWebhookRequest {
            provider: HostedProvider::GitHub,
            event: "push".to_string(),
            signature: github_signature("super-secret", &payload),
            delivery_id: Some("delivery-1".to_string()),
            shared_secret: "super-secret".to_string(),
            payload_json: payload,
        };

        let outcome = ingest_hosted_rewrite_event(request, &mut store)?;
        match outcome {
            HostedIntakeOutcome::Created(run) => {
                assert_eq!(run.repository, "acme/sce");
                assert_eq!(run.old_head, "1111111111111111111111111111111111111111");
                assert_eq!(run.new_head, "2222222222222222222222222222222222222222");
                assert!(run.idempotency_key.starts_with("hosted:github:"));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }

        assert_eq!(store.inserted.len(), 1);
        Ok(())
    }

    #[test]
    fn github_intake_rejects_invalid_signature() {
        let payload = github_payload();
        let mut store = FakeReconciliationRunStore::default();
        let request = HostedWebhookRequest {
            provider: HostedProvider::GitHub,
            event: "push".to_string(),
            signature: "sha256=deadbeef".to_string(),
            delivery_id: Some("delivery-1".to_string()),
            shared_secret: "super-secret".to_string(),
            payload_json: payload,
        };

        let error = ingest_hosted_rewrite_event(request, &mut store).expect_err("must fail");
        assert!(error
            .to_string()
            .contains("hosted event signature verification failed for github"));
        assert!(store.inserted.is_empty());
    }

    #[test]
    fn gitlab_intake_verifies_token_and_creates_run() -> Result<()> {
        let mut store = FakeReconciliationRunStore::default();
        let request = HostedWebhookRequest {
            provider: HostedProvider::GitLab,
            event: "push".to_string(),
            signature: "gitlab-secret".to_string(),
            delivery_id: Some("event-42".to_string()),
            shared_secret: "gitlab-secret".to_string(),
            payload_json: gitlab_payload(),
        };

        let outcome = ingest_hosted_rewrite_event(request, &mut store)?;
        match outcome {
            HostedIntakeOutcome::Created(run) => {
                assert_eq!(run.repository, "group/sce");
                assert_eq!(run.old_head, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
                assert_eq!(run.new_head, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
                assert!(run.idempotency_key.starts_with("hosted:gitlab:"));
            }
            other => panic!("unexpected outcome: {other:?}"),
        }

        assert_eq!(store.inserted.len(), 1);
        Ok(())
    }

    #[test]
    fn duplicate_hosted_event_is_replay_safe() -> Result<()> {
        let payload = github_payload();
        let signature = github_signature("super-secret", &payload);
        let mut store = FakeReconciliationRunStore::default();

        let first = HostedWebhookRequest {
            provider: HostedProvider::GitHub,
            event: "push".to_string(),
            signature: signature.clone(),
            delivery_id: Some("delivery-1".to_string()),
            shared_secret: "super-secret".to_string(),
            payload_json: payload.clone(),
        };
        let second = HostedWebhookRequest {
            provider: HostedProvider::GitHub,
            event: "push".to_string(),
            signature,
            delivery_id: Some("delivery-1".to_string()),
            shared_secret: "super-secret".to_string(),
            payload_json: payload,
        };

        let first_outcome = ingest_hosted_rewrite_event(first, &mut store)?;
        let second_outcome = ingest_hosted_rewrite_event(second, &mut store)?;

        assert!(matches!(first_outcome, HostedIntakeOutcome::Created(_)));
        assert!(matches!(second_outcome, HostedIntakeOutcome::Duplicate(_)));
        assert_eq!(store.inserted.len(), 2);
        Ok(())
    }

    #[test]
    fn intake_requires_before_after_and_repository_fields() {
        let payload = "{\"after\":\"2222222222222222222222222222222222222222\"}".to_string();
        let mut store = FakeReconciliationRunStore::default();
        let request = HostedWebhookRequest {
            provider: HostedProvider::GitHub,
            event: "push".to_string(),
            signature: github_signature("super-secret", &payload),
            delivery_id: Some("delivery-1".to_string()),
            shared_secret: "super-secret".to_string(),
            payload_json: payload,
        };

        let error = ingest_hosted_rewrite_event(request, &mut store).expect_err("must fail");
        assert!(error
            .to_string()
            .contains("invalid hosted event payload: missing 'before' field"));
    }

    #[test]
    fn idempotency_key_is_deterministic() {
        let key_a = derive_idempotency_key(
            HostedProvider::GitHub,
            "push",
            "acme/sce",
            "1111111111111111111111111111111111111111",
            "2222222222222222222222222222222222222222",
            Some("delivery-1"),
        );
        let key_b = derive_idempotency_key(
            HostedProvider::GitHub,
            "push",
            "acme/sce",
            "1111111111111111111111111111111111111111",
            "2222222222222222222222222222222222222222",
            Some("delivery-1"),
        );
        let key_c = derive_idempotency_key(
            HostedProvider::GitHub,
            "push",
            "acme/sce",
            "1111111111111111111111111111111111111111",
            "3333333333333333333333333333333333333333",
            Some("delivery-1"),
        );

        assert_eq!(key_a, key_b);
        assert_ne!(key_a, key_c);
    }
}
