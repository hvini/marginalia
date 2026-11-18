use crate::metadata::PdfMetadata;
use std::{fs, io, path::{Path, PathBuf}};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artigo {
    pub titulo: String,
    pub nome_arquivo: String,
    pub meta: PdfMetadata,
    pub caminho: PathBuf,
    pub tamanho_kb: u64,
}

pub fn carregar_artigos(dir: &Path) -> io::Result<Vec<Artigo>> {
    let mut artigos = Vec::new();

    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pdf")) {
                let nome = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                let tamanho_kb = fs::metadata(&path)?.len() / 1024;
                let meta = crate::metadata::extrair_metadados(&path);

                let titulo = meta.titulo.clone().unwrap_or_else(|| nome.clone());
                artigos.push(Artigo { titulo, nome_arquivo: nome, meta, caminho: path, tamanho_kb });
            }
        }
    }

    artigos.sort_by(|a, b| a.titulo.to_lowercase().cmp(&b.titulo.to_lowercase()));
    Ok(artigos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn deve_filtrar_apenas_pdfs_e_ordenar_alfabeticamente() {
        let dir = tempdir().unwrap();

        // Cria arquivos variados
        File::create(dir.path().join("zebra.pdf")).unwrap();
        let mut f = File::create(dir.path().join("Algoritmo.PDF")).unwrap();
        f.write_all(&vec![0u8; 2048]).unwrap(); // 2 KB
        File::create(dir.path().join("notas.txt")).unwrap();
        File::create(dir.path().join("imagem.png")).unwrap();

        let artigos = carregar_artigos(dir.path()).unwrap();

        assert_eq!(artigos.len(), 2);
        assert_eq!(artigos[0].titulo, "Algoritmo");
        assert_eq!(artigos[0].tamanho_kb, 2);
        assert_eq!(artigos[1].titulo, "zebra");
    }
}