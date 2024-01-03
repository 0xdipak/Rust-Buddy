// region --- Modules
mod config;

use std::{path::PathBuf};
use crate::Result;

use derive_more::{Deref, From};
use serde::{Deserialize, Serialize};
use crate::ais::{asst::{self, AsstId, ThreadId}, OaClient};

use self::config::Config;

// endregion --- Modules

const BUDDY_TOML: &str = "buddy.toml";

#[derive(Debug)]
pub struct Buddy {
    dir: PathBuf,
    oac: OaClient,
    asst_id: AsstId,
    config: Config,
}

#[derive(Debug, From, Deref, Deserialize, Serialize)]
pub struct Conv {
    thread_id: ThreadId,
}



/// Public functions
impl Buddy {
    
}

/// Private functions
impl Buddy {
    fn data_dir(&self) -> Result<PathBuf> {
        let data_dir = self.dir.join(".buddy");
        // ensure_dir(&data_dir)?;
        Ok(data_dir)
    }

    fn data_files_dir(&self) -> Result<PathBuf> {
        let dir = self.data_dir()?.join("files");
        // ensure_dir(&dir)?;
        Ok(dir)
    }
}