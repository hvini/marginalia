use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default = "default_viewer")]
    pub viewer: Option<String>,
    #[serde(default)]
    pub viewer_args: Vec<String>,
}

fn default_viewer() -> Option<String> {
    Some(String::from("zathura"))
}

impl Default for Config {
    fn default() -> Self {
        Self {
            viewer: default_viewer(),
            viewer_args: Vec::new(),
        }
    }
}

impl Config {
    pub fn caminho_config() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("pdf-viewer-cli").join("config.toml"))
    }

    pub fn carregar_ou_criar() -> (Self, Option<String>) {
        match Self::caminho_config() {
            Some(path) => Self::carregar_de_caminho(&path),
            None => (Self::default(), Some("Diretório de config não encontrado.".into())),
        }
    }

    pub fn carregar_de_caminho(path: &Path) -> (Self, Option<String>) {
        if !path.exists() {
            let cfg = Self::default();
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(path, toml::to_string_pretty(&cfg).unwrap_or_default());
            return (cfg, Some(format!("Config criada em: {}", path.display())));
        }

        match fs::read_to_string(path) {
            Ok(conteudo) => match toml::from_str::<Config>(&conteudo) {
                Ok(cfg) => (cfg, None),
                Err(e) => (Self::default(), Some(format!("Erro no config.toml: {}", e))),
            },
            Err(e) => (Self::default(), Some(format!("Erro ao ler config: {}", e))),
        }
    }

    pub fn nome_viewer_exibicao(&self) -> &str {
        self.viewer
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or("padrão do sistema")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn deve_criar_config_padrao_quando_arquivo_nao_existe() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("subpasta").join("config.toml");

        let (cfg, aviso) = Config::carregar_de_caminho(&config_path);

        assert!(config_path.exists());
        assert_eq!(cfg.viewer.as_deref(), Some("zathura"));
        assert!(aviso.unwrap().contains("Config criada"));
    }

    #[test]
    fn deve_fazer_fallback_para_default_se_toml_for_invalido() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        fs::write(&config_path, "viewer = [sintaxe quebrada").unwrap();

        let (cfg, aviso) = Config::carregar_de_caminho(&config_path);

        assert_eq!(cfg, Config::default());
        assert!(aviso.unwrap().contains("Erro no config.toml"));
    }
}