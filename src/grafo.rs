//! Grafo de citações entre os PDFs locais: A cita B se o título ou o DOI de B aparece na seção de
//! referências de A. Casamento exato sobre texto normalizado: determinístico, sem rede.

use crate::{library::Artigo, metadata::normalizar_citacao};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Grafo {
    /// `cita[i]`: índices (em `artigos`) dos artigos que i cita
    pub cita: Vec<Vec<usize>>,
    pub citado_por: Vec<Vec<usize>>,
}

/// Títulos curtos ("Introduction") casariam com qualquer texto
const MIN_PALAVRAS_TITULO: usize = 4;

// shortcut: O(n²) buscas de substring; ok para centenas de PDFs, trocar por aho-corasick se passar de milhares
pub fn construir(artigos: &[Artigo]) -> Grafo {
    // espaços nas pontas: casa só palavras inteiras ("10 1002 hbm 2585" não casa com "... 25850")
    let chaves: Vec<Vec<String>> = artigos
        .iter()
        .map(|a| {
            let titulo = a.meta.titulo.as_deref().map(normalizar_citacao).filter(|t| t.split(' ').count() >= MIN_PALAVRAS_TITULO);
            let doi = a.meta.doi.as_deref().map(normalizar_citacao).filter(|d| !d.is_empty());
            titulo.into_iter().chain(doi).map(|k| format!(" {k} ")).collect()
        })
        .collect();

    let mut g = Grafo { cita: vec![Vec::new(); artigos.len()], citado_por: vec![Vec::new(); artigos.len()] };
    for (i, a) in artigos.iter().enumerate() {
        if a.meta.referencias.is_empty() {
            continue;
        }
        let refs = format!(" {} ", a.meta.referencias);
        for (j, ch) in chaves.iter().enumerate() {
            if i != j && ch.iter().any(|k| refs.contains(k.as_str())) {
                g.cita[i].push(j);
                g.citado_por[j].push(i);
            }
        }
    }
    g
}

impl Grafo {
    pub fn conectado(&self, i: usize) -> bool {
        self.cita.get(i).is_some_and(|v| !v.is_empty()) || self.citado_por.get(i).is_some_and(|v| !v.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::PdfMetadata;
    use std::path::PathBuf;

    fn artigo(titulo: &str, doi: Option<&str>, refs: &str) -> Artigo {
        Artigo {
            titulo: titulo.into(),
            nome_arquivo: titulo.into(),
            meta: PdfMetadata {
                titulo: Some(titulo.into()),
                doi: doi.map(String::from),
                referencias: normalizar_citacao(refs),
                ..Default::default()
            },
            anotacao: Default::default(),
            caminho: PathBuf::from(format!("/b/{titulo}.pdf")),
            tamanho_kb: 1,
        }
    }

    #[test]
    fn liga_por_titulo_ou_doi_e_ignora_titulos_curtos() {
        let artigos = [
            artigo("The Hippocampal-to-Ventricle Ratio (HVR)", Some("10.1002/hbm.25850"), ""),
            artigo("Brain volumes and their ratios", None, ""),
            artigo("Survey", None, ""),
            artigo(
                "Longitudinal evaluation",
                None,
                "[1] Smith J. The hippocampal-to-ventricle ratio (HVR). NeuroImage. [2] Doe. Brain volumes and their ratios in AD. \
                 [3] X. doi:10.1002/HBM.25850 [4] Survey of things.",
            ),
            artigo("Other paper here", None, "[1] Smith. doi:10.1002/hbm.258501"),
        ];
        let g = construir(&artigos);
        assert_eq!(g.cita[3], [0, 1]); // título com pontuação diferente e DOI em maiúsculas; "Survey" curto demais
        assert_eq!(g.citado_por[0], [3]);
        assert!(g.cita[4].is_empty(), "DOI que é prefixo de outro não casa");
        assert!(!g.conectado(2) && g.conectado(1));
    }
}
