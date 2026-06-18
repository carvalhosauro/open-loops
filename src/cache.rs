//! Cache de destilações em <base>/cache/<repo>/<branch>@<head-sha>.md.
//! Chavear pelo SHA do HEAD faz o cache invalidar sozinho quando a branch anda.
use crate::scanner::OpenLoop;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Cache de destilações persistidas em disco.
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    /// Cria um `Cache` cujos arquivos ficam em `base/cache/`.
    pub fn new(base: &Path) -> Self {
        Self {
            dir: base.join("cache"),
        }
    }

    fn path(&self, lp: &OpenLoop) -> PathBuf {
        // branches têm '/', que não pode virar subdiretório no nome do arquivo
        let branch = lp.branch.replace('/', "__");
        self.dir
            .join(&lp.repo_name)
            .join(format!("{branch}@{}.md", lp.head_sha))
    }

    /// Retorna o conteúdo cacheado para `lp`, ou `None` se não existir.
    pub fn get(&self, lp: &OpenLoop) -> Option<String> {
        std::fs::read_to_string(self.path(lp)).ok()
    }

    /// Persiste `content` como destilação de `lp`.
    ///
    /// # Errors
    ///
    /// Retorna `Err` se não for possível criar os diretórios ou escrever o arquivo.
    pub fn put(&self, lp: &OpenLoop, content: &str) -> Result<()> {
        let path = self.path(lp);
        std::fs::create_dir_all(
            path.parent()
                .ok_or_else(|| anyhow::anyhow!("caminho do cache não tem diretório pai"))?,
        )?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::OpenLoop;
    use chrono::Utc;
    use std::path::PathBuf;

    fn fake_loop(sha: &str) -> OpenLoop {
        OpenLoop {
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            branch: "feat/login".into(),
            head_sha: sha.into(),
            last_commit: Utc::now(),
            ahead: 1,
            behind: 0,
        }
    }

    #[test]
    fn miss_depois_put_depois_hit() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        let lp = fake_loop("abc123");
        assert!(cache.get(&lp).is_none());
        cache.put(&lp, "contexto destilado").unwrap();
        assert_eq!(cache.get(&lp).unwrap(), "contexto destilado");
    }

    #[test]
    fn head_novo_invalida_sozinho() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = Cache::new(tmp.path());
        cache.put(&fake_loop("sha-velho"), "velho").unwrap();
        assert!(cache.get(&fake_loop("sha-novo")).is_none());
    }
}
