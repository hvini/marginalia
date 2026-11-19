//! Cache local dos metadados extraídos: um PDF só é reprocessado se mudar de tamanho ou data.

use crate::metadata::{PdfMetadata, extrair_metadados};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// Incrementar sempre que a extração mudar, para descartar resultados antigos.
const VERSAO: u32 = 2;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    versao: u32,
    #[serde(default)]
    entradas: BTreeMap<String, Entrada>, // chave: caminho canônico do PDF
    #[serde(skip)]
    alterado: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Entrada {
    tamanho: u64,
    modificado: u64, // nanossegundos desde a época
    meta: PdfMetadata,
}

impl Cache {
    pub fn caminho_padrao() -> Option<PathBuf> {
        dirs::cache_dir().map(|d| d.join("marginalia").join("metadados.toml"))
    }

    /// Arquivo ausente, corrompido ou de outra versão resulta em cache vazio.
    pub fn carregar(arquivo: &Path) -> Cache {
        fs::read_to_string(arquivo)
            .ok()
            .and_then(|s| toml::from_str::<Cache>(&s).ok())
            .filter(|c| c.versao == VERSAO)
            .unwrap_or(Cache { versao: VERSAO, ..Default::default() })
    }

    pub fn metadados(&mut self, pdf: &Path) -> PdfMetadata {
        let Some((tamanho, modificado)) = assinatura(pdf) else { return extrair_metadados(pdf) };
        let chave = pdf.canonicalize().unwrap_or_else(|_| pdf.to_path_buf()).to_string_lossy().into_owned();
        if let Some(e) = self.entradas.get(&chave).filter(|e| e.tamanho == tamanho && e.modificado == modificado) {
            return e.meta.clone();
        }
        let meta = extrair_metadados(pdf);
        self.entradas.insert(chave, Entrada { tamanho, modificado, meta: meta.clone() });
        self.alterado = true;
        meta
    }

    /// Grava só se algo mudou; remove entradas de PDFs que não existem mais.
    pub fn salvar(&mut self, arquivo: &Path) -> io::Result<()> {
        if !self.alterado {
            return Ok(());
        }
        self.entradas.retain(|caminho, _| Path::new(caminho).exists());
        let texto = toml::to_string(self).map_err(io::Error::other)?;
        if let Some(dir) = arquivo.parent() {
            fs::create_dir_all(dir)?;
        }
        // grava em arquivo temporário e renomeia: uma interrupção não corrompe o cache
        let tmp = arquivo.with_extension("toml.tmp");
        fs::write(&tmp, texto)?;
        fs::rename(tmp, arquivo)?;
        self.alterado = false;
        Ok(())
    }
}

fn assinatura(pdf: &Path) -> Option<(u64, u64)> {
    let m = fs::metadata(pdf).ok()?;
    let modificado = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64;
    Some((m.len(), modificado))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn reutiliza_entrada_e_reextrai_quando_arquivo_muda() {
        let dir = tempdir().unwrap();
        let pdf = dir.path().join("a.pdf");
        let arquivo = dir.path().join("cache").join("metadados.toml");
        fs::write(&pdf, b"nao e um pdf").unwrap();

        let mut cache = Cache::carregar(&arquivo);
        assert_eq!(cache.metadados(&pdf), PdfMetadata::default());
        // marca a entrada para provar que a próxima leitura vem do cache, não da extração
        cache.entradas.values_mut().next().unwrap().meta.titulo = Some("Do cache".into());
        cache.alterado = true;
        cache.salvar(&arquivo).unwrap();

        let mut cache = Cache::carregar(&arquivo);
        assert_eq!(cache.metadados(&pdf).titulo.as_deref(), Some("Do cache"));

        fs::write(&pdf, b"conteudo diferente e maior").unwrap();
        assert_eq!(cache.metadados(&pdf).titulo, None);
    }

    #[test]
    fn cache_corrompido_ou_de_outra_versao_e_ignorado() {
        let dir = tempdir().unwrap();
        let arquivo = dir.path().join("metadados.toml");
        fs::write(&arquivo, "lixo = [").unwrap();
        assert!(Cache::carregar(&arquivo).entradas.is_empty());
        fs::write(&arquivo, "versao = 999\n[entradas.x]\ntamanho = 1\nmodificado = 1\n[entradas.x.meta]\npaginas = 3\n").unwrap();
        assert!(Cache::carregar(&arquivo).entradas.is_empty());
    }
}
