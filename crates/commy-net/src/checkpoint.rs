use std::collections::BTreeMap;

use commy_core::identity::{Identity, IdentityKey, Signature};
use commy_core::{ServerId, ServerState, UserId};
use commy_sync::StreamKeys;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::community_streams::STATE_TAIL;
use crate::protocol::{build_sync_response_within, sign_sync_response, WirePayload};

pub const CHECKPOINT_VERSION: u16 = 1;
pub const STREAM_LABEL: &str = "commy-community-checkpoint/v1";
const SEAL_DOMAIN: &str = "commy-checkpoint-seal/v1";
pub const CHUNK_BYTES: usize = 60 * 1024;
pub const MAX_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_CHUNKS: u32 = (MAX_CHECKPOINT_BYTES / CHUNK_BYTES) as u32 + 1;
pub const FIRST_LOOK: u32 = 8;
const MAX_HELD_BYTES: usize = 2 * MAX_CHECKPOINT_BYTES;

pub fn keys_for(state: &StreamKeys) -> StreamKeys {
    state.sibling(STREAM_LABEL)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Record {
    version: u16,
    at: i64,
    part: Part,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Part {
    Chunk { index: u32, bytes: Vec<u8> },
    Sealed { count: u32, covers: u64, digest: [u8; 32], sig: Signature },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    pub snapshot: WirePayload,
    pub covers: u64,
    pub at: i64,
}

impl Checkpoint {
    pub fn state_after(&self) -> u64 {
        state_after(self.covers)
    }
}

pub fn state_after(covers: u64) -> u64 {
    covers.saturating_sub(u64::from(STATE_TAIL))
}

fn seal_message(server: ServerId, at: i64, count: u32, covers: u64, digest: &[u8; 32]) -> Vec<u8> {
    postcard::to_stdvec(&(SEAL_DOMAIN, server, at, count, covers, digest)).expect("plain values always encode")
}

pub fn snapshot_of(state: &ServerState, by: UserId, at: i64, identity: &Identity) -> WirePayload {
    let mut snapshot = build_sync_response_within(state, 0, by, MAX_CHECKPOINT_BYTES);
    sign_sync_response(&mut snapshot, at, identity);
    snapshot
}

pub fn records(snapshot: &WirePayload, covers: u64, identity: &Identity) -> Option<Vec<Vec<u8>>> {
    let WirePayload::SyncResponse { server, at, .. } = snapshot else { return None };
    let bytes = commy_core::wire_layout::encode(snapshot).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_CHECKPOINT_BYTES {
        return None;
    }
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    let count = bytes.len().div_ceil(CHUNK_BYTES) as u32;
    let sig = identity.sign(&seal_message(server.id, *at, count, covers, &digest));
    let chunks = bytes.chunks(CHUNK_BYTES).enumerate().map(|(index, bytes)| Part::Chunk { index: index as u32, bytes: bytes.to_vec() });
    let parts = chunks.chain(std::iter::once(Part::Sealed { count, covers, digest, sig }));
    parts.map(|part| postcard::to_stdvec(&Record { version: CHECKPOINT_VERSION, at: *at, part }).ok()).collect()
}

struct Seal {
    seq: u64,
    count: u32,
    covers: u64,
    digest: [u8; 32],
    sig: Signature,
}

#[derive(Default)]
pub struct Assembler {
    chunks: BTreeMap<i64, BTreeMap<u32, Vec<u8>>>,
    seals: BTreeMap<i64, Seal>,
    first_seq: Option<u64>,
    held_bytes: usize,
}

impl Assembler {
    pub fn take(&mut self, seq: u64, plain: &[u8]) {
        self.first_seq = Some(self.first_seq.map_or(seq, |first| first.min(seq)));
        let Ok(record) = postcard::from_bytes::<Record>(plain) else { return };
        if record.version != CHECKPOINT_VERSION {
            return;
        }
        match record.part {
            Part::Chunk { index, bytes } => {
                let fits = index < MAX_CHUNKS && bytes.len() <= CHUNK_BYTES && self.held_bytes + bytes.len() <= MAX_HELD_BYTES;
                if fits && !self.chunks.get(&record.at).is_some_and(|held| held.contains_key(&index)) {
                    self.held_bytes += bytes.len();
                    self.chunks.entry(record.at).or_default().insert(index, bytes);
                }
            }
            Part::Sealed { count, covers, digest, sig } if count > 0 && count <= MAX_CHUNKS => {
                self.seals.insert(record.at, Seal { seq, count, covers, digest, sig });
            }
            Part::Sealed { .. } => {}
        }
    }

    pub fn read_again_from(&self) -> Option<u64> {
        let (at, seal) = self.seals.iter().next_back()?;
        let held = self.chunks.get(at).map_or(0, BTreeMap::len);
        if held >= seal.count as usize {
            return None;
        }
        let start = seal.seq.saturating_sub(2 * u64::from(seal.count) + 2);
        self.first_seq.is_some_and(|first| first > start + 1).then_some(start)
    }

    pub fn newest(&self, server: ServerId, owner: &IdentityKey) -> Option<Checkpoint> {
        self.seals.iter().rev().find_map(|(at, seal)| self.open(server, owner, *at, seal))
    }

    fn open(&self, server: ServerId, owner: &IdentityKey, at: i64, seal: &Seal) -> Option<Checkpoint> {
        let chunks = self.chunks.get(&at)?;
        let mut bytes = Vec::new();
        for index in 0..seal.count {
            bytes.extend_from_slice(chunks.get(&index)?);
        }
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        if digest != seal.digest || !owner.verify(&seal_message(server, at, seal.count, seal.covers, &digest), &seal.sig) {
            return None;
        }
        let snapshot: WirePayload = commy_core::wire_layout::decode(&bytes).ok()?;
        let matches = matches!(&snapshot, WirePayload::SyncResponse { server: copy, at: signed_at, .. } if copy.id == server && *signed_at == at);
        if !matches {
            return None;
        }
        Some(Checkpoint { snapshot, covers: seal.covers, at })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::verify_sync_response;
    use commy_core::test_support::{fixture, identity_for, join};
    use commy_sync::StreamSecret;

    fn big_server(members: usize) -> (ServerState, UserId) {
        let (mut state, _, owner) = fixture();
        for i in 0..members {
            join(&mut state, &format!("member-{i}"));
        }
        (state, owner)
    }

    fn assembled(records: &[Vec<u8>], first_seq: u64) -> Assembler {
        let mut assembler = Assembler::default();
        for (i, plain) in records.iter().enumerate() {
            assembler.take(first_seq + i as u64, plain);
        }
        assembler
    }

    fn members_in(payload: &WirePayload) -> usize {
        let WirePayload::SyncResponse { members, .. } = payload else { return 0 };
        members.len()
    }

    #[test]
    fn a_big_server_checkpoint_keeps_every_member_and_still_carries_the_owners_signature() {
        let (state, owner) = big_server(5_000);
        let identity = identity_for(owner);
        let snapshot = snapshot_of(&state, owner, 1_760_000_000_000, &identity);
        assert_eq!(members_in(&snapshot), state.server.members.len());
        let records = records(&snapshot, 4_321, &identity).unwrap();
        assert!(records.len() > 2, "the copy is spread over several entries");
        assert!(records.iter().all(|plain| plain.len() <= commy_sync::stream::MAX_PLAIN));
        let found = assembled(&records, 40).newest(state.server.id, &identity.public()).unwrap();
        assert_eq!(found.covers, 4_321);
        assert_eq!(found.snapshot, snapshot);
        assert!(verify_sync_response(&found.snapshot, &identity.public()));
    }

    #[test]
    fn only_the_newest_complete_checkpoint_by_the_owner_is_taken() {
        let (state, owner) = big_server(20);
        let identity = identity_for(owner);
        let older = records(&snapshot_of(&state, owner, 100, &identity), 7, &identity).unwrap();
        let newer = records(&snapshot_of(&state, owner, 200, &identity), 9, &identity).unwrap();
        let mut all: Vec<Vec<u8>> = older.clone();
        all.extend(newer.iter().cloned());
        assert_eq!(assembled(&all, 1).newest(state.server.id, &identity.public()).unwrap().at, 200);
        let mut unfinished = older;
        unfinished.extend(newer[..newer.len() - 1].iter().cloned());
        assert_eq!(assembled(&unfinished, 1).newest(state.server.id, &identity.public()).unwrap().at, 100, "a copy cut off mid-write is skipped");
        assert!(assembled(&all, 1).newest(ServerId::new(), &identity.public()).is_none(), "another server");
    }

    #[test]
    fn a_checkpoint_written_by_anyone_but_the_owner_is_refused() {
        let (mut state, owner) = big_server(3);
        let member = join(&mut state, "sneaky");
        let forged = records(&snapshot_of(&state, owner, 300, &identity_for(member)), 1, &identity_for(member)).unwrap();
        assert!(assembled(&forged, 1).newest(state.server.id, &identity_for(owner).public()).is_none());
        let mut tampered = records(&snapshot_of(&state, owner, 300, &identity_for(owner)), 1, &identity_for(owner)).unwrap();
        let first = tampered[0].len() - 1;
        tampered[0][first] ^= 1;
        assert!(assembled(&tampered, 1).newest(state.server.id, &identity_for(owner).public()).is_none());
        let mut assembler = assembled(&tampered, 1);
        assembler.take(99, b"junk");
        assert!(assembler.newest(state.server.id, &identity_for(owner).public()).is_none());
    }

    #[test]
    fn a_reader_that_saw_only_the_end_of_a_long_checkpoint_reads_further_back_once() {
        let (state, owner) = big_server(5_000);
        let identity = identity_for(owner);
        let records = records(&snapshot_of(&state, owner, 500, &identity), 1, &identity).unwrap();
        let first_seq = 1_000;
        let seen = records.len() - FIRST_LOOK as usize;
        let mut tail = Assembler::default();
        for (i, plain) in records.iter().enumerate().skip(seen) {
            tail.take(first_seq + i as u64, plain);
        }
        let start = tail.read_again_from().unwrap();
        assert!(start < first_seq, "it starts before the first chunk");
        assert!(tail.newest(state.server.id, &identity.public()).is_none());
        let whole = assembled(&records, first_seq);
        assert_eq!(whole.read_again_from(), None);
        assert!(whole.newest(state.server.id, &identity.public()).is_some());
        assert_eq!(Assembler::default().read_again_from(), None, "an empty stream has nothing to look back for");
    }

    #[test]
    fn the_state_stream_starts_early_enough_to_cover_everything_after_the_checkpoint() {
        assert_eq!(state_after(0), 0);
        assert_eq!(state_after(u64::from(STATE_TAIL)), 0);
        assert_eq!(state_after(u64::from(STATE_TAIL) + 50), 50);
    }

    #[test]
    fn each_state_stream_has_its_own_checkpoint_stream() {
        let state = StreamSecret::from_bytes([1; 32]).keys();
        let other = StreamSecret::from_bytes([2; 32]).keys();
        assert_eq!(keys_for(&state).id, keys_for(&StreamSecret::from_bytes([1; 32]).keys()).id);
        assert_ne!(keys_for(&state).id, state.id);
        assert_ne!(keys_for(&state).id, keys_for(&other).id);
    }
}
