//! Loops descartados pelo usuário ("não vale continuar").
//! Persistido em <base>/ignores.toml, chaves no formato "repo/branch".
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Serialize, Deserialize)]
struct IgnoreFile {
    #[serde(default)]
    ignored: BTreeSet<String>,
}

pub struct Ignores {
    path: PathBuf,
    set: BTreeSet<String>,
}

impl Ignores {
    /// Carrega a lista de ignorados a partir de `<base>/ignores.toml`.
    ///
    /// Arquivo ausente é tratado como lista vazia.
    ///
    /// # Errors
    ///
    /// Retorna erro se o arquivo existir mas não puder ser lido ou não for
    /// TOML válido no formato esperado.
    pub fn load(base: &Path) -> Result<Self> {
        let path = base.join("ignores.toml");
        let set = match std::fs::read_to_string(&path) {
            Ok(raw) => {
                toml::from_str::<IgnoreFile>(&raw)
                    .with_context(|| format!("ignores.toml inválido em {}", path.display()))?
                    .ignored
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(e) => return Err(e).context(format!("lendo {}", path.display())),
        };
        Ok(Self { path, set })
    }

    /// Adiciona `key` à lista de ignorados e persiste em disco imediatamente.
    ///
    /// # Errors
    ///
    /// Retorna erro se o diretório base não puder ser criado, se o caminho
    /// não tiver diretório pai, ou se a escrita do arquivo falhar.
    pub fn add(&mut self, key: &str) -> Result<()> {
        self.set.insert(key.to_string());
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("caminho sem diretório pai: {}", self.path.display()))?;
        std::fs::create_dir_all(parent)?;
        let file = IgnoreFile {
            ignored: self.set.clone(),
        };
        std::fs::write(&self.path, toml::to_string_pretty(&file)?)?;
        Ok(())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.set.contains(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vazio_quando_arquivo_nao_existe() {
        let tmp = tempfile::tempdir().unwrap();
        let ig = Ignores::load(tmp.path()).unwrap();
        assert!(!ig.contains("repo/branch"));
    }

    #[test]
    fn add_persiste_e_contains_acha() {
        let tmp = tempfile::tempdir().unwrap();
        let mut ig = Ignores::load(tmp.path()).unwrap();
        ig.add("app/feat/x").unwrap();
        let recarregado = Ignores::load(tmp.path()).unwrap();
        assert!(recarregado.contains("app/feat/x"));
        assert!(!recarregado.contains("app/feat/y"));
    }
}
