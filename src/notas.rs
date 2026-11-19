//! Notas em Markdown por artigo: `<biblioteca>/notas/<nome do PDF>.md`, criadas a partir de um modelo
//! preenchido com os metadados extraídos. Nunca sobrescreve uma nota existente.

use crate::library::Artigo;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const PASTA: &str = "notas";

pub fn caminho(dir_notas: &Path, a: &Artigo) -> PathBuf {
    dir_notas.join(format!("{}.md", a.nome_arquivo))
}

/// Cria a nota com o modelo se ainda não existir; devolve o caminho.
pub fn garantir(dir_notas: &Path, a: &Artigo) -> io::Result<PathBuf> {
    let nota = caminho(dir_notas, a);
    fs::create_dir_all(dir_notas)?;
    match OpenOptions::new().write(true).create_new(true).open(&nota) {
        Ok(mut f) => f.write_all(modelo(a).as_bytes())?,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    Ok(nota)
}

pub fn modelo(a: &Artigo) -> String {
    let m = &a.meta;
    let publicacao: Vec<String> = [
        m.venue.clone(),
        m.volume.as_ref().map(|v| format!("vol. {v}")),
        m.intervalo_paginas.as_ref().map(|p| format!("pp. {p}")),
        m.ano.clone(),
    ]
    .into_iter()
    .flatten()
    .collect();
    let arquivo_pdf = a.caminho.file_name().unwrap_or_default().to_string_lossy();

    let campos = [
        ("Autores", m.autores.clone()),
        ("Publicação", (!publicacao.is_empty()).then(|| publicacao.join(" · "))),
        ("DOI", m.doi.as_ref().map(|d| format!("[{d}](https://doi.org/{d})"))),
        ("arXiv", m.arxiv.as_ref().map(|x| format!("[{x}](https://arxiv.org/abs/{x})"))),
        // caminho relativo: a nota continua válida se a pasta da biblioteca for movida
        ("PDF", Some(format!("[{arquivo_pdf}](<../{arquivo_pdf}>)"))),
        ("Palavras-chave", m.palavras_chave.clone()),
    ];

    let mut s = format!("# {}\n\n", a.titulo);
    for (rotulo, valor) in campos {
        if let Some(v) = valor {
            s.push_str(&format!("**{rotulo}:** {v}  \n"));
        }
    }
    if let Some(r) = &m.resumo {
        s.push_str(&format!("\n## Resumo\n\n> {r}\n"));
    }
    s.push_str("\n## Notas\n\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::PdfMetadata;

    fn artigo() -> Artigo {
        Artigo {
            titulo: "Deep Nets".into(),
            nome_arquivo: "deep".into(),
            meta: PdfMetadata {
                autores: Some("Ana Silva; John Doe".into()),
                venue: Some("NeuroImage".into()),
                ano: Some("2019".into()),
                doi: Some("10.1/x".into()),
                resumo: Some("Um resumo.".into()),
                ..Default::default()
            },
            anotacao: Default::default(),
            caminho: PathBuf::from("/bib/deep.pdf"),
            tamanho_kb: 1,
        }
    }

    #[test]
    fn modelo_preenchido_com_metadados() {
        let s = modelo(&artigo());
        assert!(s.starts_with("# Deep Nets\n\n**Autores:** Ana Silva; John Doe  \n**Publicação:** NeuroImage · 2019  \n"));
        assert!(s.contains("**DOI:** [10.1/x](https://doi.org/10.1/x)"));
        assert!(s.contains("**PDF:** [deep.pdf](<../deep.pdf>)"));
        assert!(s.contains("## Resumo\n\n> Um resumo.\n"));
        assert!(s.ends_with("## Notas\n\n"));
        assert!(!s.contains("arXiv"));
    }

    #[test]
    fn garantir_cria_uma_vez_e_preserva_edicoes() {
        let dir = tempfile::tempdir().unwrap();
        let notas = dir.path().join(PASTA);
        let nota = garantir(&notas, &artigo()).unwrap();
        assert_eq!(nota, notas.join("deep.md"));
        fs::write(&nota, "minhas anotações").unwrap();
        garantir(&notas, &artigo()).unwrap();
        assert_eq!(fs::read_to_string(&nota).unwrap(), "minhas anotações");
    }
}
