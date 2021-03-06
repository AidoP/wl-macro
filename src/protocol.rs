use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use serde::Deserialize;

// Note: owned strings are required as TOML allows string normalisation

#[derive(Debug, Deserialize)]
pub struct Protocol {
    pub name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub copyright: Option<String>,
    #[serde(rename = "interface", default)]
    pub interfaces: Vec<Interface>
}
impl Protocol {
    fn default_path(named: &str) -> PathBuf {
        let mut path = if let Some(path) = option_env!("WL_PROTOCOLS") {
            PathBuf::from(path)
        } else {
            PathBuf::from("protocol/")
        };
        path.push(named);
        path.set_extension("toml");
        path
    }
    pub fn from_str(string: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(string)
    }
    pub fn load<P: AsRef<Path>>(named: &str) -> Self {
        let mut protocol = String::new();
        let path = &Self::default_path(named);
        let mut file = File::open(path).unwrap_or_else(|error| panic!("Unable to open protocol specification file {:?}: {:?}", path, error));
        file.read_to_string(&mut protocol).unwrap_or_else(|error| panic!("Unable to read protocol specification file {:?}: {:?}", path, error));
        Self::from_str(&protocol).unwrap_or_else(|error| panic!("Failed to parse protocol specification file {:?}: {:?}", path, error))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Interface {
    pub name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub version: u16,
    #[serde(rename = "enum", default)]
    pub enums: Vec<Enum>,
    #[serde(rename = "request", default)]
    pub requests: Vec<Request>,
    #[serde(rename = "event", default)]
    pub events: Vec<Event>
}

#[derive(Clone, Debug, Deserialize)]
pub struct Enum {
    pub name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub since: Option<u16>,
    #[serde(rename = "entry", default)]
    pub entries: Vec<Entry>
}
#[derive(Clone, Debug, Deserialize)]
pub struct Request {
    pub name: String,
    pub since: Option<u16>,
    #[serde(default)]
    pub destructor: bool,
    pub summary: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "arg", default)]
    pub args: Vec<Arg>
}
#[derive(Clone, Debug, Deserialize)]
pub struct Event {
    pub name: String,
    pub since: Option<u16>,
    pub summary: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "arg", default)]
    pub args: Vec<Arg>
}

#[derive(Clone, Debug, Deserialize)]
pub struct Entry {
    pub name: String,
    pub since: Option<u16>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub value: u32
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestType {
    Destructor
}

#[derive(Clone, Debug, Deserialize)]
pub struct Arg {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: DataType,
    pub interface: Option<String>,
    #[serde(rename = "enum")]
    pub enumeration: Option<String>,
    pub summary: Option<String>
}

#[derive(Copy, Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataType {
    Int,
    Uint,
    Fixed,
    String,
    Array,
    Fd,
    Object,
    NewId
}