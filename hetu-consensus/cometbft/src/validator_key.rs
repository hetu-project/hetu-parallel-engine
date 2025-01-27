use std::{fs, io, path::Path};
use serde::{Deserialize, Serialize};
use cometbft::{
    PrivateKey, PublicKey, Signature,
    account::Id,
    block::{Block, Id as BlockId, CommitSig},
    block::parts::Header as PartSetHeader,
    proposal::{Proposal, SignProposalRequest},
    chain::Id as ChainId,
};
use cometbft_proto::types::v1::{Proposal as RawProposal};
use ed25519_consensus::{SigningKey, VerificationKey};

#[derive(Serialize, Deserialize)]
pub struct FilePVKey {
    pub address: Id,
    pub pub_key: PublicKey,
    pub priv_key: PrivateKey,
}

pub struct PrivValidatorKey {
    pub key: FilePVKey,
    pub chain_id: String,
}

impl Clone for PrivValidatorKey {
    fn clone(&self) -> Self {
        Self {
            key: FilePVKey {
                address: self.key.address.clone(),
                pub_key: self.key.pub_key.clone(),
                priv_key: match &self.key.priv_key {
                    PrivateKey::Ed25519(key) => {
                        let cloned_key = key.clone();
                        PrivateKey::Ed25519(cloned_key)
                    }
                    _ => panic!("only Ed25519 keys supported"),
                },
            },
            chain_id: self.chain_id.clone(),
        }
    }
}

impl PrivValidatorKey {
    pub fn load_or_generate(key_file: &Path, chain_id: &str) -> io::Result<Self> {
        if key_file.exists() {
            let contents = fs::read_to_string(key_file)?;
            let key: FilePVKey = serde_json::from_str(&contents)?;
            Ok(Self { key, chain_id: chain_id.to_string() })
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "validator key file not found"))
        }
    }

    pub fn sign_proposal(&mut self, proposal: &mut RawProposal) -> Result<(), io::Error> {
        // Convert to cometbft proposal type for canonical signing
        let canonical_proposal = Proposal::try_from(proposal.clone())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        
        // Create sign request
        let chain_id = ChainId::try_from(self.chain_id.clone())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let sign_req = SignProposalRequest {
            proposal: canonical_proposal,
            chain_id,
        };
        
        // Get sign bytes
        let sign_bytes = sign_req.into_signable_vec();

        // Sign with Ed25519
        let signature = match &self.key.priv_key {
            PrivateKey::Ed25519(key) => {
                let signing_key = SigningKey::try_from(key.as_bytes())
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid signing key"))?;
                let sig = signing_key.sign(&sign_bytes);
                Signature::try_from(sig.to_bytes().as_slice())
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
            }
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "only Ed25519 keys supported")),
        };

        // Update signature in proposal
        proposal.signature = signature.as_bytes().to_vec();
        Ok(())
    }

    pub fn sign_block(&mut self, block: &mut Block) -> Result<(), io::Error> {
        // Create proposal from block
        let mut proposal = RawProposal {
            r#type: 32, // ProposalType::Proposal
            height: block.header.height.value() as i64,
            round: 0, // Always 0 for block proposals
            pol_round: -1, // No POL for block proposals
            block_id: Some(BlockId {
                hash: block.header.hash(),
                part_set_header: PartSetHeader::default(),
            }.into()),
            timestamp: Some(block.header.time.into()),
            signature: Vec::new(),
        };

        // Sign the proposal
        self.sign_proposal(&mut proposal)?;

        // Update block signatures only for non-genesis blocks
        if block.last_commit.is_some() {
            if let Some(commit) = &mut block.last_commit {
                // Add validator signature to commit
                let sig = CommitSig::BlockIdFlagCommit {
                    validator_address: self.key.address.clone(),
                    timestamp: block.header.time,
                    signature: Some(Signature::try_from(proposal.signature.as_slice())
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?),
                };
                commit.signatures = vec![sig];
            }
        }

        Ok(())
    }

    pub fn sign(&self, msg: &[u8]) -> Signature {
        match &self.key.priv_key {
            PrivateKey::Ed25519(key) => {
                let signing_key = SigningKey::try_from(key.as_bytes())
                    .expect("invalid signing key");
                let sig = signing_key.sign(msg);
                Signature::try_from(sig.to_bytes().as_slice())
                    .expect("invalid signature")
            }
            _ => panic!("only Ed25519 keys supported"),
        }
    }

    pub fn pub_key(&self) -> &PublicKey {
        &self.key.pub_key
    }
}
