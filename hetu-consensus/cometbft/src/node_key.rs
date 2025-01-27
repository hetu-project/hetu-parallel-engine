use cometbft::account::Id;
use cometbft::private_key::{self, PrivateKey};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

#[derive(Serialize, Deserialize)]
pub struct NodeKey {
    #[serde(rename = "priv_key")]
    pub priv_key: PrivateKey,
}

impl NodeKey {
    pub fn new() -> Self {
        let mut rng = OsRng;
        let mut seed = [0u8; 32];
        rng.fill_bytes(&mut seed);
        let priv_key = PrivateKey::Ed25519(private_key::Ed25519::try_from(&seed[..]).unwrap());
        Self { priv_key }
    }

    pub fn id(&self) -> String {
        let pub_key = self.priv_key.public_key();
        let id = Id::from(pub_key);
        hex::encode(id.as_bytes())
    }

    pub fn save_as(&self, file_path: &Path) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(file_path, json)?;
        Ok(())
    }

    pub fn load_or_generate(file_path: &Path) -> io::Result<Self> {
        if file_path.exists() {
            let json = fs::read_to_string(file_path)?;
            let key = serde_json::from_str(&json)?;
            Ok(key)
        } else {
            let key = Self::new();
            key.save_as(file_path)?;
            Ok(key)
        }
    }
}
