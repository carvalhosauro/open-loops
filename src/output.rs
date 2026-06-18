//! Renderização para o terminal: tabela do inventário e idades humanas.
use crate::scanner::OpenLoop;
use chrono::{DateTime, Utc};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::OpenLoop;
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
}
