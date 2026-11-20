use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub theme: String,
    pub hosts: Vec<String>,
    pub clients: Vec<String>,
    pub additional_processes: Vec<String>,
    pub selected_profile: Option<String>,
    pub helper_path: String,
    pub certificate_trust_confirmed: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            hosts: vec![],
            clients: vec!["desktop".into(), "terminal".into()],
            additional_processes: vec![],
            selected_profile: None,
            helper_path: String::new(),
            certificate_trust_confirmed: false,
        }
    }
}

pub fn normalize_host(input: &str) -> Result<String, String> {
    let raw = input.trim();
    if raw.is_empty() || raw.len() > 512 {
        return Err("Укажите домен сервера.".into());
    }
    let url = url::Url::parse(&if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    })
    .map_err(|_| format!("Некорректный адрес сервера: {raw}"))?;
    if !["https", "http"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Используйте домен или HTTP(S)-адрес без логина и пароля.".into());
    }
    url.host_str()
        .map(|host| {
            host.trim_matches(['[', ']'])
                .trim_end_matches('.')
                .to_lowercase()
        })
        .filter(|host| !host.is_empty())
        .ok_or_else(|| "В адресе отсутствует домен.".into())
}

pub fn normalized_processes(processes: &[String]) -> Result<Vec<String>, String> {
    if processes.len() > 32 {
        return Err("Можно добавить до 32 процессов.".into());
    }
    let mut names = BTreeSet::new();
    for process in processes {
        let name = process.trim();
        if name.is_empty() {
            continue;
        }
        if name.len() > 128
            || name
                .chars()
                .any(|c| c.is_control() || matches!(c, ',' | '!' | '/' | '\\'))
            || name.chars().all(|c| c.is_ascii_digit())
        {
            return Err("Укажите имя процесса, без пути, PID и символов ! или запятой.".into());
        }
        names.insert(name.to_owned());
    }
    Ok(names.into_iter().collect())
}

pub fn selector(processes: &[String]) -> Result<String, String> {
    let mut names = BTreeSet::from(["Claude".to_owned(), "claude".to_owned()]);
    names.extend(normalized_processes(processes)?);
    Ok(names.into_iter().collect::<Vec<_>>().join(","))
}
