
// region: --- Modules

pub mod asst;
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

