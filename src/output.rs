//! Renderização para o terminal: tabela do inventário e idades humanas.
use crate::scanner::OpenLoop;
use crate::worktrees::{Verdict, Worktree};
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::path::PathBuf;

/// Converte a diferença entre `now` e `then` em string legível por humanos.
///
/// - `< 60 min` → `"{N}min"`
/// - `< 48 h`   → `"{N}h"`
/// - `≥ 48 h`   → `"{N}d"`
pub fn human_age(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
    let mins = (now - then).num_minutes().max(0);
    if mins < 60 {
        format!("{mins}min")
    } else if mins < 48 * 60 {
        format!("{}h", mins / 60)
    } else {
        format!("{}d", mins / (60 * 24))
    }
}

/// Renderiza uma tabela do inventário de loops abertos ordenada do mais parado
/// para o mais recente (o critério de atenção é o staleness).
///
/// Retorna uma mensagem comemorativa quando a lista está vazia.
pub fn render_table(loops: &[OpenLoop], now: DateTime<Utc>) -> String {
    if loops.is_empty() {
        return "Nenhum loop aberto. Tudo finalizado ou ignorado.\n".into();
    }
    let mut sorted: Vec<&OpenLoop> = loops.iter().collect();
    sorted.sort_by_key(|l| l.last_commit);
    let key_w = sorted
        .iter()
        .map(|l| l.key().len())
        .max()
        .unwrap_or(4)
        .max(4);
    let mut out = format!(
        "{:<key_w$}  {:>9}  {:>5}  {:>6}\n",
        "LOOP", "PARADO HÁ", "AHEAD", "BEHIND"
    );
    for l in sorted {
        out.push_str(&format!(
            "{:<key_w$}  {:>9}  {:>5}  {:>6}\n",
            l.key(),
            human_age(now, l.last_commit),
            l.ahead,
            l.behind
        ));
    }
    out
}

fn verdict_rank(v: &Verdict) -> u8 {
    match v {
        Verdict::Deletable | Verdict::Prunable => 0,
        Verdict::Cold => 1,
        Verdict::Active => 2,
        Verdict::Home => 3,
    }
}

fn branch_label(w: &Worktree) -> String {
    w.branch.clone().unwrap_or_else(|| "(detached)".into())
}

/// Renders the worktree table + ASCII cleanup-command block.
///
/// Order: deletable/prunable first, then oldest idle first.
pub fn render_worktrees(wts: &[Worktree], now: DateTime<Utc>) -> String {
    if wts.is_empty() {
        return "No worktrees found.\n".into();
    }
    let epoch = DateTime::from_timestamp(0, 0).unwrap();
    let mut sorted: Vec<&Worktree> = wts.iter().collect();
    sorted.sort_by_key(|w| (verdict_rank(&w.verdict()), w.last_commit.unwrap_or(epoch)));

    let name_w = sorted.iter().map(|w| w.short_name().len()).max().unwrap_or(8).max(8);
    let branch_w = sorted.iter().map(|w| branch_label(w).len()).max().unwrap_or(6).max(6);

    let mut out = format!(
        "{:<name_w$}  {:<branch_w$}  {:>5}  {:>6}  {:>5}  {}\n",
        "WORKTREE", "BRANCH", "IDLE", "MERGED", "STATE", "VERDICT"
    );
    for w in &sorted {
        out.push_str(&format!(
            "{:<name_w$}  {:<branch_w$}  {:>5}  {:>6}  {:>5}  {}\n",
            w.short_name(),
            branch_label(w),
            w.last_commit.map(|t| human_age(now, t)).unwrap_or_else(|| "?".into()),
            if w.merged { "yes" } else { "no" },
            if w.dirty { "dirty" } else { "clean" },
            w.verdict().label()
        ));
    }

    let mut cmds: Vec<String> = Vec::new();
    let mut pruned: HashSet<PathBuf> = HashSet::new();
    for w in &sorted {
        match w.verdict() {
            Verdict::Deletable => {
                if let Some(b) = &w.branch {
                    cmds.push(format!(
                        "git -C {repo} worktree remove {wt} && git -C {repo} branch -d {b}",
                        repo = w.repo_path.display(),
                        wt = w.worktree_path.display(),
                    ));
                }
            }
            Verdict::Prunable => {
                if pruned.insert(w.repo_path.clone()) {
                    cmds.push(format!("git -C {} worktree prune", w.repo_path.display()));
                }
            }
            _ => {}
        }
    }
    if cmds.is_empty() {
        out.push_str("\n# nothing to clean up.\n");
    } else {
        out.push_str(&format!("\n# {} worktree(s) to clean up. Copy to run:\n", cmds.len()));
        for c in &cmds {
            out.push_str(c);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::OpenLoop;
    use crate::worktrees::Worktree;
    use chrono::{Duration, Utc};
    use std::path::PathBuf;

    fn lp(branch: &str, idade_dias: i64) -> OpenLoop {
        OpenLoop {
            repo_name: "app".into(),
            repo_path: PathBuf::from("/tmp/app"),
            branch: branch.into(),
            head_sha: "abc".into(),
            last_commit: Utc::now() - Duration::days(idade_dias),
            ahead: 1,
            behind: 0,
        }
    }

    #[test]
    fn human_age_minutos_horas_dias() {
        let now = Utc::now();
        assert_eq!(human_age(now, now - Duration::minutes(5)), "5min");
        assert_eq!(human_age(now, now - Duration::hours(3)), "3h");
        assert_eq!(human_age(now, now - Duration::days(12)), "12d");
    }

    #[test]
    fn render_table_ordena_mais_parado_primeiro() {
        let t = render_table(&[lp("recente", 1), lp("antiga", 30)], Utc::now());
        let pos_antiga = t.find("antiga").unwrap();
        let pos_recente = t.find("recente").unwrap();
        assert!(pos_antiga < pos_recente);
        assert!(t.contains("LOOP"));
        assert!(t.contains("30d"));
    }

    #[test]
    fn render_table_vazia_celebra() {
        assert!(render_table(&[], Utc::now()).contains("Nenhum loop aberto"));
    }

    fn wt(branch: &str, merged: bool, dirty: bool, idade_dias: i64) -> Worktree {
        Worktree {
            repo_name: "app".into(),
            repo_path: std::path::PathBuf::from("/tmp/app"),
            worktree_path: std::path::PathBuf::from(format!("/tmp/app/{branch}")),
            branch: Some(branch.into()),
            last_commit: Some(Utc::now() - Duration::days(idade_dias)),
            merged,
            dirty,
            prunable: false,
            is_main: false,
        }
    }

    #[test]
    fn render_worktrees_sorts_deletable_first_and_shows_command() {
        let out = render_worktrees(
            &[
                wt("feat/cold", false, false, 40),
                wt("fix/done", true, false, 8),
            ],
            Utc::now(),
        );
        // header ASCII
        assert!(out.contains("WORKTREE"));
        assert!(out.contains("VERDICT"));
        // deletable aparece antes de cold
        let pos_done = out.find("fix/done").unwrap();
        let pos_cold = out.find("feat/cold").unwrap();
        assert!(pos_done < pos_cold);
        // bloco de comando para a deletable
        assert!(out.contains("worktree remove"));
        assert!(out.contains("branch -d fix/done"));
        // ASCII-only
        assert!(out.is_ascii());
    }

    #[test]
    fn render_worktrees_no_action_says_nothing() {
        let out = render_worktrees(&[wt("feat/cold", false, false, 3)], Utc::now());
        assert!(out.contains("nothing to clean up"));
    }

    #[test]
    fn render_worktrees_empty() {
        assert!(render_worktrees(&[], Utc::now()).contains("No worktrees found"));
    }
}
