
// ==== file path: buddy\../src\ais\asst.rs

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;

use crate::ais::msg::get_text_content;
use crate::ais::{msg::user_msg, OaClient};
use crate::utils::cli::{ico_check, ico_deleted_ok, ico_err, ico_uploaded, ico_uploading};
use crate::utils::files::XFile;
use crate::Result;
use async_openai::types::{
    CreateAssistantFileRequest, CreateFileRequest, CreateRunRequest, RunStatus,
};
use console::Term;
use derive_more::{Deref, Display, From};

use async_openai::{
    config::OpenAIConfig,
    types::{
        AssistantObject, AssistantToolsRetrieval, CreateAssistantRequest, CreateThreadRequest,
        ModifyAssistantRequest, ThreadObject,
    },
    Assistants,
};

// region: --- Constants
const DEFAULT_QUERY: &[(&str, &str)] = &[("limit", "100")];
const POLLING_DURATION_MS: u64 = 500;
// endregion: --- Constants

// region: --- Types

pub struct CreateConfig {
    pub name: String,
    pub model: String,
}

#[derive(Debug, From, Deref, Display)]
pub struct AsstId(String);

#[derive(Debug, From, Deref, Display, Serialize, Deserialize)]
pub struct ThreadId(String);

#[derive(Debug, From, Deref, Display)]
pub struct FileId(String);

// endregion: --- Types

// region: --- Asst CRUD
pub async fn create(oac: &OaClient, config: CreateConfig) -> Result<AsstId> {
    let oa_assts: Assistants<'_, OpenAIConfig> = oac.assistants();

    let asst_obj: AssistantObject = oa_assts
        .create(CreateAssistantRequest {
            model: config.model,
            name: Some(config.name),
            tools: Some(vec![AssistantToolsRetrieval::default().into()]),
            ..Default::default()
        })
        .await?;

    Ok(asst_obj.id.into())
}

pub async fn load_or_create_asst(
    oac: &OaClient,
    config: CreateConfig,
    recreate: bool,
) -> Result<AsstId> {
    let asst_obj = first_by_name(oac, &config.name).await?;
    let mut asst_id = asst_obj.map(|o| AsstId::from(o.id));

    // -- Delete asst if recreate is true and asst_id
    if let (true, Some(asst_id_ref)) = (recreate, asst_id.as_ref()) {
        delete(oac, asst_id_ref).await?;
        asst_id.take();
        println!("{} Assistant {} deleted", ico_deleted_ok(), config.name);
    }
    // -- Create if needed
    if let Some(asst_id) = asst_id {
        println!("{} Assistant {} loaded", ico_check(), config.name);
        Ok(asst_id)
    } else {
        let asst_name = config.name.clone();
        let asst_id = create(oac, config).await?;
        println!("{} Assistant {} loaded", ico_check(), asst_name);
        Ok(asst_id)
    }
}

pub async fn first_by_name(oac: &OaClient, name: &str) -> Result<Option<AssistantObject>> {
    let oa_assts = oac.assistants();

    let assts = oa_assts.list(DEFAULT_QUERY).await?.data;

    let asst_obj = assts
        .into_iter()
        .find(|a| a.name.as_ref().map(|n| n == name).unwrap_or(false));

    Ok(asst_obj)
}

pub async fn upload_instructions(
    oac: &OaClient,
    asst_id: &AsstId,
    inst_content: String,
) -> Result<()> {
    let oa_assts = oac.assistants();
    let modif = ModifyAssistantRequest {
        instructions: Some(inst_content),
        ..Default::default()
    };

    oa_assts.update(asst_id, modif).await?;

    Ok(())
}

pub async fn delete(oac: &OaClient, asst_id: &AsstId) -> Result<()> {
    let oa_assts = oac.assistants();
    let oa_files = oac.files();

    // First delete the files associated to this assistant.
    for file_id in get_file_hashmap(oac, asst_id).await?.into_values() {
        let del_res = oa_files.delete(&file_id).await;
        // Might be already deleted, that's ok for now.
        if del_res.is_ok() {
            println!("{} file deleted - {file_id}", ico_deleted_ok());
        }
    }

    // No need to delete assistant files since we delete the assistant.
    


    // -- Delete assistant
    oa_assts.delete(asst_id).await?;

    Ok(())
}

// endregion: --- Asst CRUD

// region: --- Thread

pub async fn create_thread(oac: &OaClient) -> Result<ThreadId> {
    let oa_threads = oac.threads();

    let res = oa_threads
        .create(CreateThreadRequest {
            ..Default::default()
        })
        .await?;

    Ok(res.id.into())
}

pub async fn get_thread(oac: &OaClient, thread_id: &ThreadId) -> Result<ThreadObject> {
    let oa_threads = oac.threads();

    let thread_obj = oa_threads.retrieve(thread_id).await?;

    Ok(thread_obj)
}

pub async fn run_thread_msg(
    oac: &OaClient,
    asst_id: &AsstId,
    thread_id: &ThreadId,
    msg: &str,
) -> Result<String> {
    let msg = user_msg(msg);

    // -- Attach message to thread
    let _message_obj = oac.threads().messages(thread_id).create(msg).await?;

    // -- Create a run for the thread
    let run_request = CreateRunRequest {
        assistant_id: asst_id.to_string(),
        ..Default::default()
    };

    let run = oac.threads().runs(thread_id).create(run_request).await?;

    // -- Loop to get result
    let term = Term::stdout();
    loop {
        term.write_str(">")?;
        let run = oac.threads().runs(thread_id).retrieve(&run.id).await?;
        term.write_str("<")?;

        match run.status {
            RunStatus::Completed => {
                term.write_str("\n")?;
                return get_first_thread_msg_content(oac, thread_id).await;
            }
            RunStatus::Queued | RunStatus::InProgress => (),
            other => {
                term.write_str("\n")?;
                return Err(format!("ERROR WHILE RUN: {:?}", other).into());
            }
        }
        sleep(Duration::from_millis(POLLING_DURATION_MS)).await;
    }
}

pub async fn get_first_thread_msg_content(oac: &OaClient, thread_id: &ThreadId) -> Result<String> {
    static QUERY: [(&str, &str); 1] = [("limit", "1")];

    let messages = oac.threads().messages(thread_id).list(&QUERY).await?;
    let msg = messages
        .data
        .into_iter()
        .next()
        .ok_or_else(|| "No message found".to_string())?;

    let text = get_text_content(msg)?;

    Ok(text)
}

// endregion --- Thread

// region: --- Files

/// returns the file id by file name hashmap.
pub async fn get_file_hashmap(oac: &OaClient, asst_id: &AsstId) -> Result<HashMap<String, FileId>> {
    // get all asst files (files do not have .name)
    let oa_assts = oac.assistants();
    let oa_asst_files = oa_assts.files(asst_id);
    let asst_files = oa_asst_files.list(DEFAULT_QUERY).await?.data;
    let asst_file_ids: HashSet<String> = asst_files.into_iter().map(|f| f.id).collect();

    // Get all files for org (those files have .filename)
    let oa_files = oac.files();
    let org_files = oa_files.list().await?.data; // need changes

    // Build or file_name:file_id hashmap
    let file_id_by_name: HashMap<String, FileId> = org_files
        .into_iter()
        .filter(|org_file| asst_file_ids.contains(&org_file.id))
        .map(|org_file| (org_file.filename, org_file.id.into()))
        .collect();

    Ok(file_id_by_name)
}

/// Uploads a file to an assistant (dirst to the account, then attaches to asst)
pub async fn upload_file_by_name(
    oac: &OaClient,
    asst_id: &AsstId,
    file: &Path,
    force: bool,
) -> Result<(FileId, bool)> {
    let file_name = file.x_file_name();
    let mut file_id_by_name = get_file_hashmap(oac, asst_id).await?;

    let file_id = file_id_by_name.remove(file_name);

    // If not force and file already created, return early.
    if !force {
        if let Some(file_id) = file_id {
            return Ok((file_id, false));
        }
    }

    // if we have old file_id, we delete the file.
    if let Some(file_id) = file_id {
        // Delete the org file
        let oa_files = oac.files();
        if let Err(err) = oa_files.delete(&file_id).await {
            println!(
                "{} Can't delete file '{}'\n  cause: {}",
                ico_err(),
                file.to_string_lossy(),
                err
            );
        }

        // Delete the asst_file association
        let oa_assts = oac.assistants();
        let oa_assts_files = oa_assts.files(asst_id);
        if let Err(err) = oa_assts_files.delete(&file_id).await {
            println!(
                "{} Can't remove assistant file '{}'\n  cause: {}",
                ico_err(),
                file.x_file_name(),
                err
            );
        }
    }

    // Upload and attach the file
    let term = Term::stdout();

    // Print uploading
    term.write_line(&format!(
        "{} Uploading file '{}'",
        ico_uploading(),
        file.x_file_name()
    ))?;

    // Upload file
    let oa_files = oac.files();
    let oa_file = oa_files
        .create(CreateFileRequest {
            file: file.into(),
            purpose: "assistants".into(),
        })
        .await?;

    // Update print
    term.clear_last_lines(1)?;
    term.write_line(&format!(
        "{} Uploaded file '{}'",
        ico_uploaded(),
        file.x_file_name()
    ))?;

    // Attach file to assistant
    let oa_assts = oac.assistants();
    let oa_assts_files = oa_assts.files(asst_id);
    let asst_file_obj = oa_assts_files
        .create(CreateAssistantFileRequest {
            file_id: oa_file.id.clone(),
        })
        .await?;

    // Assert warning
    if oa_file.id != asst_file_obj.id {
        println!(
            "SHOULD NOT HAPPEN, File id not matching {} {}",
            oa_file.id, asst_file_obj.id
        )
    }

    Ok((asst_file_obj.id.into(), true))
}
// endregion: --- Files




// ==== file path: buddy\../src\ais\mod.rs


// region: --- Modules

pub mod asst;
pub mod msg;
use crate::Result;
use dotenv;


// use crate::utils::files::get_glob_set;
// use crate::Result;
use async_openai::config::OpenAIConfig;
use async_openai::Client;

// endregion: --- Modules



// region: --- Client

// const ENV_OPENAI_API_KEY: &str = "OPENAI_API_KEY";

pub type OaClient = Client<OpenAIConfig>;

pub fn new_oa_client() -> Result<OaClient> {
	if dotenv::var("OPENAI_API_KEY").is_ok(){
		Ok(Client::new())
	} else {
		println!("No ENV_OPENAI_API_KEY env variable. Please set it.");

		Err("No openai api key in env".into())
	}
}

// endregion: --- Client




// ==== file path: buddy\../src\ais\msg.rs

use async_openai::types::{CreateMessageRequest, MessageObject, MessageContent};

use crate::Result;


// region --- Message Constructors

pub fn user_msg(content: impl Into<String>) -> CreateMessageRequest {
    CreateMessageRequest {
        role: "user".to_string(),
        content: content.into(),
        ..Default::default()
    }
}

// endregion --- Message Constructors


// region --- Content Constructor

pub fn get_text_content(msg: MessageObject) -> Result<String> {
    // -- Get the first content item
    let msg_content = msg
    .content
    .into_iter()
    .next()
    .ok_or_else(|| "No message content found".to_string())?;

    // -- Get the text
    let txt = match  msg_content {
        MessageContent::Text(text) => text.text.value,
        MessageContent::ImageFile(_) => {
            return Err("Message image not supported yet".into());
        }
    };

    Ok(txt)
}


// endregion --- Content Constructor




// ==== file path: buddy\../src\buddy\config.rs

use serde::Deserialize;

use crate::ais::asst;



#[derive(Debug, Deserialize)]

pub(super) struct  Config {
    pub name: String,
    pub model: String,
    pub instructions_file: String,
    pub file_bundles: Vec<FileBundle>,
}


#[derive(Debug, Deserialize)]

pub(super) struct FileBundle {
    pub bundle_name: String,
    pub src_dir: String,
    pub dst_ext: String,
    pub src_globs: Vec<String>,
}


// region --- Froms

impl From<&Config> for asst::CreateConfig {
    fn from(config: &Config) -> Self {
        Self {
            name: config.name.clone(),
            model: config.model.clone(),
        }
    }
}

// endregion --- Froms




// ==== file path: buddy\../src\buddy\mod.rs

// region --- Modules
mod config;

use crate::{
    ais::new_oa_client,
    utils::{
        cli::ico_check,
        files::{
            bundle_to_file, ensure_dir, list_files, load_from_json, load_from_toml, read_to_string,
            save_to_json,
        },
    },
    Result,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::ais::{
    asst::{self, AsstId, ThreadId},
    OaClient,
};
use derive_more::{Deref, From};
use serde::{Deserialize, Serialize};

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
    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub async fn init_form_dir(dir: impl AsRef<Path>, recreate_asst: bool) -> Result<Self> {
        let dir = dir.as_ref();

        // load from directory
        let config: Config = load_from_toml(dir.join(BUDDY_TOML))?;

        // Get or create the openAI assistant
        let oac = new_oa_client()?;
        let asst_id = asst::load_or_create_asst(&oac, (&config).into(), recreate_asst).await?;

        // Create buddy
        let buddy = Buddy {
            dir: dir.to_path_buf(),
            oac,
            asst_id,
            config,
        };

        // Upload the instructions
        buddy.upload_instructions().await?;

        // Upload the file
        buddy.upload_files(false).await?;

        Ok(buddy)
    }

    pub async fn upload_instructions(&self) -> Result<bool> {
        let file = self.dir.join(&self.config.instructions_file);
        if file.exists() {
            let inst_content = read_to_string(&file)?;
            asst::upload_instructions(&self.oac, &self.asst_id, inst_content).await?;
            println!("{} Instructions uploaded", ico_check());
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn upload_files(&self, recreate: bool) -> Result<u32> {
        let mut num_uploaded = 0;

        // the .buddy/files
        let data_files_dir = self.data_files_dir()?;

        // Clean the .buddy/files left over.
        let exclude_element = format!("*{}*", &self.asst_id);
        for file in list_files(
            &data_files_dir,
            Some(&["*.rs", "*.md"]),
            Some(&[&exclude_element]),
        )? {
            // delete file
            let file_str = file.to_string_lossy();
            // Safeguard
            if !file_str.contains(".buddy") {
                return Err(format!("Error should no delete: '{}'", file_str).into());
            }
            fs::remove_file(&file)?;
        }

        // Genrate and upload the .buddy/files bundle files
        for bundle in self.config.file_bundles.iter() {
            let src_dir = self.dir.join(&bundle.src_dir);

            if src_dir.is_dir() {
                let src_globs: Vec<&str> = bundle.src_globs.iter().map(AsRef::as_ref).collect();

                let files = list_files(&src_dir, Some(&src_globs), None)?;

                if !files.is_empty() {
                    // Compute bundle file name.
                    let bundle_file_name = format!(
                        "{}-{}-bundle-{}.{}",
                        self.name(),
                        bundle.bundle_name,
                        self.asst_id,
                        bundle.dst_ext
                    );

                    let bundle_file = self.data_files_dir()?.join(bundle_file_name);

                    // If it does not exist, then we will force a reupload.
                    let force_reupload = recreate || !bundle_file.exists();

                    // Rebundle no matter if exist or not (to check)
                    bundle_to_file(files, &bundle_file)?;

                    // Upload
                    let (_, uploaded) = asst::upload_file_by_name(
                        &self.oac,
                        &self.asst_id,
                        &bundle_file,
                        force_reupload,
                    )
                    .await?;

                    if uploaded {
                        num_uploaded += 1;
                    }
                }
            }
        }

        Ok(num_uploaded)
    }

    pub async fn load_or_create_conv(&self, recreate: bool) -> Result<Conv> {
        let conv_file = self.data_dir()?.join("conv.json");

        if recreate && conv_file.exists() {
            let _ = fs::remove_file(&conv_file);
        }
        let conv = if let Ok(conv) = load_from_json::<Conv>(&conv_file) {
            asst::get_thread(&self.oac, &conv.thread_id)
                .await
                .map_err(|_| format!("Connot find thread_id for {:?}", conv))?;
            println!("{} Conversation loaded", ico_check());
            conv
        } else {
            let thread_id = asst::create_thread(&self.oac).await?;
            println!("{} Conversation created", ico_check());
            let conv = thread_id.into();
            save_to_json(&conv_file, &conv)?;
            conv
        };

        Ok(conv)
    }

    pub async fn chat(&self, conv: &Conv, msg: &str) -> Result<String> {
        let res = asst::run_thread_msg(&self.oac, &self.asst_id, &conv.thread_id, msg).await?;

        Ok(res)
    }
}

/// Private functions
impl Buddy {
    fn data_dir(&self) -> Result<PathBuf> {
        let data_dir = self.dir.join(".buddy");
        ensure_dir(&data_dir)?;
        Ok(data_dir)
    }

    fn data_files_dir(&self) -> Result<PathBuf> {
        let dir = self.data_dir()?.join("files");
        ensure_dir(&dir)?;
        Ok(dir)
    }
}




// ==== file path: buddy\../src\error.rs

pub type Result<T> = core::result::Result<T, Error>;

pub type Error = Box<dyn std::error::Error>;




// ==== file path: buddy\../src\main.rs

// region: --- Modules

mod ais;
// mod buddy;
mod buddy;
mod error;
mod utils;

// use ais::new_oa_client;
use textwrap::wrap;

use crate::{ buddy::Buddy, utils::cli::{prompt, ico_res, text_res, ico_err}};

pub use self::error::{Error, Result};

// endregion: --- Modules

#[tokio::main]
async fn main() {
    println!();

    match start().await {
        Ok(_) => println!("\nBye!\n"),
        Err(e) => println!("\nError: {}\n", e),
    }
}

const DEFAULT_DIR: &str = "buddy";

// region: --- Types

/// Input Command from user

#[derive(Debug)]
enum Cmd {
    Quit,
    Chat(String),
    RefreshAll,
    RefreshConv,
    RefreshInst,
    RefreshFiles
}

impl Cmd {
    fn from_input(input: impl Into<String>) -> Self {
        let input = input.into();

        if input == "/q" {
			Self::Quit
		} else if input == "/r" || input == "/ra" {
			Self::RefreshAll
		} else if input == "/ri" {
			Self::RefreshInst
		} else if input == "/rf" {
			Self::RefreshFiles
		} else if input == "/rc" {
			Self::RefreshConv
		} else {
			Self::Chat(input)
		}
    }
}
// endregion: --- Types


async fn start() -> Result<()> {
    let  buddy = Buddy::init_form_dir(DEFAULT_DIR, false).await?;

    let  conv = buddy.load_or_create_conv(false).await?;

    loop {
        println!();
        let input = prompt("Ask away")?;
        let cmd = Cmd::from_input(input);

        match cmd {
            Cmd::Quit => break,
            Cmd::Chat(msg) => {
                let res = buddy.chat(&conv, &msg).await?;
                let res = wrap(&res, 80).join("\n");
                println!("{} {}", ico_res(), text_res(res));
            },
            other => println!("{} command not supported {other:?}", ico_err()),
        }
    }


    println!("->> buddy {} - conv {conv:?}", buddy.name());

    Ok(())
}




// ==== file path: buddy\../src\utils\cli.rs

use console::{Style, style, StyledObject};
use dialoguer::{Input, theme::ColorfulTheme};

use crate::Result;


// region: --- Prompts

pub fn prompt(text: &str) -> Result<String> {
    let theme = ColorfulTheme {
        prompt_style: Style::new().for_stderr().color256(45),
        prompt_prefix: style("?".to_string()).color256(45).for_stderr(),
        ..ColorfulTheme::default()
    };

    let input = Input::with_theme(&theme);
    let res = input.with_prompt(text).interact_text()?;

    Ok(res)
}

// endregion: --- Prompts



// region: --- Icons

pub fn ico_res() -> StyledObject<&'static str> {
	style("➤").color256(45)
}

pub fn ico_check() -> StyledObject<&'static str> {
	style("✔").green()
}

pub fn ico_uploading() -> StyledObject<&'static str> {
	style("↥").yellow()
}

pub fn ico_uploaded() -> StyledObject<&'static str> {
	style("↥").green()
}

pub fn ico_deleted_ok() -> StyledObject<&'static str> {
	style("⌫").green()
}

pub fn ico_err() -> StyledObject<&'static str> {
	style("✗").red()
}


// endregion: --- Icons



// region: --- Text Output

pub fn text_res(text: String) -> StyledObject<String> {
    style(text).bright()
}

// endregion: --- Text Output




// ==== file path: buddy\../src\utils\files.rs

use std::{
    fs::{self, File},
    path::{Path, PathBuf}, io::{BufReader, BufWriter, Write, BufRead}, ffi::OsStr,
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use walkdir::WalkDir;

use crate::Result;

// region: --- Fille Bundler

pub fn bundle_to_file(files: Vec<PathBuf>, dst_file: &Path) -> Result<()> {
    let mut writer = BufWriter::new(File::create(dst_file)?);


    for file in files {
        if !file.is_file() {
            return  Err(format!("Connot bundle '{:?}' is not a file.", file).into());
        }
        let reader = get_reader(&file)?;

        writeln!(writer, "\n// ==== file path: {}\n", file.to_string_lossy())?;

        for line in reader.lines() {
            let line = line?;
            writeln!(writer, "{}", line)?;
        }
        writeln!(writer, "\n\n")?;
    }
    writer.flush()?;

    Ok(())
}

// endregion: --- Fille Bundler

// region: --- File Parser/Writer

pub fn load_from_toml<T>(file: impl AsRef<Path>) -> Result<T> 
    where
    T: serde::de::DeserializeOwned,
    {
        let content = read_to_string(file.as_ref())?;

        Ok(toml::from_str(&content)?)
    }

pub fn load_from_json<T>(file: impl AsRef<Path>) -> Result<T>
where
    T: serde::de::DeserializeOwned, {
        let val = serde_json::from_reader(get_reader(file.as_ref())?)?;
        Ok(val)
    }


pub fn save_to_json<T>(file: impl AsRef<Path>, data: &T) -> Result<()>
where
    T: serde::Serialize,
    {
        let file = file.as_ref();

        let file = File::create(file)
        .map_err(|e| format!("Can not create file '{:?}' : {}", file, e))?;
    serde_json::to_writer_pretty(file, data)?;

    Ok(())
    }

// endregion: --- File Parser/Writer


// region: --- Dir Utils

// Returns true if one or more dir was created
pub fn ensure_dir(dir: &Path) -> Result<bool> {
    if dir.is_dir() {
        Ok(false)
    } else {
        fs::create_dir_all(dir)?;
        Ok(true)
    }
}

pub fn list_files(
    dir: &Path,
    include_globs: Option<&[&str]>,
    exclude_globs: Option<&[&str]>,
) -> Result<Vec<PathBuf>> {
    let base_dir_exclude: GlobSet = base_dir_exclude_globs()?;

    // Determine Recursive depth
    let depth = include_globs
        .map(|globs| globs.iter().any(|&g| g.contains("**")))
        .map(|v| if v { 100 } else { 1 })
        .unwrap_or(1);

    // Prep globs
    let include_globs = include_globs.map(get_glob_set).transpose()?;
    let exclude_globs = exclude_globs.map(get_glob_set).transpose()?;

    // Build file iterator
    let walk_dir_it = WalkDir::new(dir)
    .max_depth(depth)
    .into_iter()
    .filter_entry(|e| 
        // if dir check dir exclude
        if e.file_type().is_dir() {
            !base_dir_exclude.is_match(e.path())
        } 
        // else file, we apply the globs
        else {
            // first evaluate the exclude
            if let Some(exclude_globs) = exclude_globs.as_ref() {
                if exclude_globs.is_match(e.path()) {
                    return  false;
                }
            }
            // otherwise, evaluate the include
            match include_globs.as_ref() {
                Some(globs) => globs.is_match(e.path()),
                None => true,
            }
        }
    )
    .filter_map(|e| e.ok().filter(|e| e.file_type().is_file()));

    let paths = walk_dir_it.map(|e| e.into_path());

    Ok(paths.collect())
}

fn base_dir_exclude_globs() -> Result<GlobSet> {
    get_glob_set(&["**/.git", "**/target"])
}

pub fn get_glob_set(globs: &[&str]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for glob in globs {
        builder.add(Glob::new(glob)?);
    }
    Ok(builder.build()?)
}

// endregion: --- Dir Utils


// region: --- File Utills

pub fn read_to_string(file: &Path) -> Result<String> {
    if !file.is_file() {
        return Err(format!("Fille not found: {}", file.display()).into());
    }
    let content = fs::read_to_string(file)?;

    Ok(content)
}

fn get_reader(file: &Path) -> Result<BufReader<File>> {
    let Ok(file) = File::open(file) else {
        return Err(format!("File not found: {}", file.display()).into());
    };

    Ok(BufReader::new(file))
}

// endregion: --- File Utils



// region --- XFile

/// Trait that has methods that returns
/// the `&str` when ok, and when none or err, returns ""
pub trait XFile {
    fn x_file_name(&self) -> &str;
    fn x_extension(&self) -> &str;
}


impl XFile for Path {
    fn x_file_name(&self) -> &str {
        self.file_name().and_then(OsStr::to_str).unwrap_or("")
    }

    fn x_extension(&self) -> &str {
        self.extension().and_then(OsStr::to_str).unwrap_or("")
    }
}

// endregion --- XFile




// ==== file path: buddy\../src\utils\mod.rs


// region --- Modules

pub mod files;
pub mod cli;


// endregion --- Modules



