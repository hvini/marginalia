// src/opener.rs
use std::{
    env,
    path::Path,
    process::{Command, Stdio},
};
use crate::config::Config;

/// Trait que abstrai como um PDF é aberto (permite mocks nos testes)
pub trait PdfOpener {
    fn open(&self, caminho: &Path, config: &Config) -> Result<String, String>;
}

/// Implementação real usada em produção
pub struct SystemOpener;

impl PdfOpener for SystemOpener {
    fn open(&self, caminho: &Path, config: &Config) -> Result<String, String> {
        let viewer_escolhido = env::var("PDF_VIEWER")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| config.viewer.clone().filter(|v| !v.trim().is_empty()));

        if let Some(viewer) = viewer_escolhido {
            let res = Command::new(&viewer)
                .args(&config.viewer_args)
                .arg(caminho)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();

            if res.is_ok() {
                return Ok(viewer);
            }
        }

        open::that_detached(caminho)
            .map(|_| String::from("padrão do sistema (fallback)"))
            .map_err(|e| e.to_string())
    }
}