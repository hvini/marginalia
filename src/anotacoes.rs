//! Status de leitura e tags definidos pelo usuário. Ao contrário do cache, são dados que não
//! se regeneram: ficam em `.marginalia.toml` na pasta da biblioteca, indexados pelo nome do PDF.

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::Path,
};

pub const NOME_ARQUIVO: &str = ".marginalia.toml";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Status {
    #[default]
    #[serde(rename = "para ler")]
    ParaLer,
    #[serde(rename = "lendo")]
    Lendo,
    #[serde(rename = "lido")]
    Lido,
}

impl Status {
    pub fn nome(self) -> &'static str {
        match self {
            Status::ParaLer => "Para ler",
            Status::Lendo => "Lendo",
            Status::Lido => "Lido",
        }
    }

    pub fn proximo(self) -> Status {
        match self {
            Status::ParaLer => Status::Lendo,
            Status::Lendo => Status::Lido,
            Status::Lido => Status::ParaLer,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Anotacao {
    pub status: Status,
    pub tags: Vec<String>,
}

pub type Anotacoes = BTreeMap<String, Anotacao>;

/// Chave de um PDF: nome do arquivo.
// shortcut: renomear o PDF perde status/tags; trocar por hash do conteúdo se isso incomodar.
pub fn chave(caminho: &Path) -> String {
    caminho.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// Arquivo ausente = biblioteca sem anotações. Arquivo ilegível é erro: quem chama não deve sobrescrevê-lo.
pub fn carregar(arquivo: &Path) -> io::Result<Anotacoes> {
    match fs::read_to_string(arquivo) {
        Ok(s) => toml::from_str(&s).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Anotacoes::new()),
        Err(e) => Err(e),
    }
}

/// Grava de forma atômica (temporário + rename); entradas sem nada além do padrão são omitidas.
pub fn salvar(arquivo: &Path, anotacoes: &Anotacoes) -> io::Result<()> {
    let uteis: Anotacoes = anotacoes.iter().filter(|(_, a)| **a != Anotacao::default()).map(|(k, a)| (k.clone(), a.clone())).collect();
    let texto = toml::to_string(&uteis).map_err(io::Error::other)?;
    let tmp = arquivo.with_extension("toml.tmp");
    fs::write(&tmp, texto)?;
    fs::rename(tmp, arquivo)
}

/// "ML, deep learning, ml" → ["ML", "deep-learning"]: sem espaços (o filtro separa termos por espaço), sem repetidas
pub fn parse_tags(texto: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for t in texto.split(',').map(|t| t.split_whitespace().collect::<Vec<_>>().join("-")).filter(|t| !t.is_empty()) {
        if !tags.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
            tags.push(t);
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salva_e_carrega_omitindo_entradas_padrao() {
        let dir = tempfile::tempdir().unwrap();
        let arquivo = dir.path().join(NOME_ARQUIVO);
        assert!(carregar(&arquivo).unwrap().is_empty());

        let mut a = Anotacoes::new();
        a.insert("a.pdf".into(), Anotacao { status: Status::Lido, tags: vec!["ml".into()] });
        a.insert("b.pdf".into(), Anotacao::default());
        salvar(&arquivo, &a).unwrap();

        let lido = carregar(&arquivo).unwrap();
        assert_eq!(lido.len(), 1);
        assert_eq!(lido["a.pdf"].status, Status::Lido);
        assert!(fs::read_to_string(&arquivo).unwrap().contains("status = \"lido\""));
    }

    #[test]
    fn arquivo_corrompido_e_erro() {
        let dir = tempfile::tempdir().unwrap();
        let arquivo = dir.path().join(NOME_ARQUIVO);
        fs::write(&arquivo, "lixo = [").unwrap();
        assert!(carregar(&arquivo).is_err());
    }

    #[test]
    fn tags_sem_espacos_e_sem_duplicatas() {
        assert_eq!(parse_tags("ML, deep learning, ml, ,"), ["ML", "deep-learning"]);
    }
}
