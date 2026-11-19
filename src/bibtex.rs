//! Exportação para BibTeX a partir dos metadados extraídos.

use crate::library::Artigo;
use std::{collections::HashSet, fs::OpenOptions, io::{self, Write}, path::Path};

/// Grava as entradas em `destino`. Recusa sobrescrever um arquivo existente (pode ter sido editado à mão).
pub fn exportar(artigos: &[&Artigo], destino: &Path) -> io::Result<()> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(destino)?;
    f.write_all(gerar(artigos).as_bytes())
}

pub fn gerar(artigos: &[&Artigo]) -> String {
    let mut chaves = HashSet::new();
    artigos.iter().map(|a| entrada(a, &mut chaves)).collect::<Vec<_>>().join("\n")
}

fn entrada(a: &Artigo, chaves: &mut HashSet<String>) -> String {
    let m = &a.meta;
    let conferencia = m.venue.as_deref().is_some_and(|v| {
        let v = v.to_lowercase();
        ["proceedings", "conference", "symposium", "workshop", "congress"].iter().any(|p| v.contains(p))
    });
    let (tipo, campo_venue) = match &m.venue {
        Some(v) if v == "arXiv" => ("misc", None),
        Some(_) if conferencia => ("inproceedings", Some("booktitle")),
        Some(_) => ("article", Some("journal")),
        None => ("misc", None),
    };

    let campos: Vec<(&str, Option<String>)> = vec![
        ("title", Some(a.titulo.clone())),
        ("author", m.autores.as_ref().map(|s| s.split(';').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" and "))),
        (campo_venue.unwrap_or("howpublished"), m.venue.clone()),
        ("year", m.ano.clone()),
        ("volume", m.volume.clone()),
        ("pages", m.intervalo_paginas.as_ref().map(|p| p.replace('–', "--"))),
        ("doi", m.doi.clone()),
        ("eprint", m.arxiv.clone()),
        ("archiveprefix", m.arxiv.as_ref().map(|_| "arXiv".into())),
        ("keywords", m.palavras_chave.as_ref().map(|k| k.replace("; ", ", "))),
        ("abstract", m.resumo.clone()),
        ("file", Some(a.caminho.to_string_lossy().into_owned())),
    ];
    let corpo: Vec<String> = campos
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| format!("  {k} = {{{}}}", if k == "file" { v } else { escapar(&v) })))
        .collect();
    format!("@{tipo}{{{},\n{}\n}}\n", chave(a, chaves), corpo.join(",\n"))
}

/// "coupe2022hippocampal"; colisões ganham sufixo a, b, c…
fn chave(a: &Artigo, usadas: &mut HashSet<String>) -> String {
    let ascii = |s: &str| -> String { s.chars().filter_map(sem_acento).filter(char::is_ascii_alphanumeric).collect::<String>().to_lowercase() };
    let autor = a.meta.autores.as_deref().and_then(|s| s.split(';').next()?.split_whitespace().last()).map(ascii).unwrap_or_default();
    let palavra = a.titulo.split_whitespace().map(ascii).find(|w| w.len() > 3).unwrap_or_default();
    let base = format!("{autor}{}{palavra}", a.meta.ano.as_deref().unwrap_or(""));
    let base = if base.is_empty() { ascii(&a.nome_arquivo) } else { base };
    let base = if base.is_empty() { "ref".to_string() } else { base };
    let mut chave = base.clone();
    for sufixo in 'a'..='z' {
        if !usadas.contains(&chave) {
            break;
        }
        chave = format!("{base}{sufixo}");
    }
    // shortcut: mais de 27 colisões da mesma chave repetem o último sufixo; improvável numa biblioteca pessoal
    usadas.insert(chave.clone());
    chave
}

fn sem_acento(c: char) -> Option<char> {
    const DE: &str = "áàâãäåçéèêëíìîïñóòôõöøúùûüýÿÁÀÂÃÄÅÇÉÈÊËÍÌÎÏÑÓÒÔÕÖØÚÙÛÜÝ";
    const PARA: &str = "aaaaaaceeeeiiiinoooooouuuuyyAAAAAACEEEEIIIINOOOOOOUUUUY";
    Some(DE.chars().position(|d| d == c).and_then(|i| PARA.chars().nth(i)).unwrap_or(c))
}

/// Caracteres especiais do LaTeX; chaves removidas para nunca desbalancear a entrada
fn escapar(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '{' | '}' => {}
            '&' | '%' | '$' | '#' | '_' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::PdfMetadata;
    use std::path::PathBuf;

    fn artigo(titulo: &str, meta: PdfMetadata) -> Artigo {
        Artigo { titulo: titulo.into(), nome_arquivo: "arq".into(), meta, anotacao: Default::default(), caminho: PathBuf::from("/p/a.pdf"), tamanho_kb: 1 }
    }

    #[test]
    fn gera_article_com_campos_e_escapes() {
        let a = artigo("Deep & Wide {Nets}", PdfMetadata {
            autores: Some("Pierrick Coupé; José V. Manjon".into()),
            venue: Some("Human Brain Mapping".into()),
            ano: Some("2022".into()),
            intervalo_paginas: Some("3270–3282".into()),
            doi: Some("10.1002/hbm.25850".into()),
            ..Default::default()
        });
        let bib = gerar(&[&a]);
        assert!(bib.starts_with("@article{coupe2022deep,\n"));
        assert!(bib.contains("  title = {Deep \\& Wide Nets},"));
        assert!(bib.contains("  author = {Pierrick Coupé and José V. Manjon},"));
        assert!(bib.contains("  journal = {Human Brain Mapping},"));
        assert!(bib.contains("  pages = {3270--3282},"));
        assert!(bib.contains("  file = {/p/a.pdf}\n}"));
    }

    #[test]
    fn tipo_por_venue_e_chaves_unicas() {
        let conf = artigo("A Paper Title", PdfMetadata { venue: Some("Proceedings of the ACM SIGMOD".into()), ano: Some("2024".into()), ..Default::default() });
        let arxiv = artigo("A Paper Title", PdfMetadata { venue: Some("arXiv".into()), arxiv: Some("2101.00001".into()), ano: Some("2024".into()), ..Default::default() });
        let bib = gerar(&[&conf, &arxiv]);
        assert!(bib.contains("@inproceedings{2024paper,\n  title = {A Paper Title},\n  booktitle = {Proceedings of the ACM SIGMOD}"));
        assert!(bib.contains("@misc{2024papera,"));
        assert!(bib.contains("  eprint = {2101.00001},\n  archiveprefix = {arXiv}"));
    }

    #[test]
    fn nao_sobrescreve_arquivo_existente() {
        let dir = tempfile::tempdir().unwrap();
        let destino = dir.path().join("refs.bib");
        std::fs::write(&destino, "editado à mão").unwrap();
        let a = artigo("Titulo Qualquer", PdfMetadata::default());
        assert_eq!(exportar(&[&a], &destino).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&destino).unwrap(), "editado à mão");
    }
}
