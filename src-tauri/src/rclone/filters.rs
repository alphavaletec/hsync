//! Converte os filtros amigáveis da UI em regras `FilterRule` do rclone.
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filters {
    /// Nomes de pastas a ignorar em qualquer nível (ou a partir da raiz, se começarem com `/`).
    pub exclude_dirs: Vec<String>,
    /// Extensões a ignorar (`tmp`, `.tmp` ou `*.tmp`).
    pub exclude_exts: Vec<String>,
    /// Ignora dotfiles e arquivos de sistema do Windows.
    pub exclude_hidden: bool,
    /// Regras cruas do rclone (`+ padrão` / `- padrão`), aplicadas primeiro.
    pub custom_rules: Vec<String>,
}

const HIDDEN_RULES: &[&str] = &[
    "- .*",
    "- .*/**",
    "- desktop.ini",
    "- Thumbs.db",
    "- $RECYCLE.BIN/**",
    "- System Volume Information/**",
];

impl Filters {
    pub fn validate(&self) -> AppResult<()> {
        for r in &self.custom_rules {
            let r = r.trim();
            if !(r.starts_with("+ ") || r.starts_with("- ")) || r.len() < 3 {
                return Err(AppError::invalid(format!(
                    "Regra inválida \"{r}\": use \"+ padrão\" ou \"- padrão\""
                )));
            }
        }
        for d in &self.exclude_dirs {
            if d.trim().trim_matches('/').is_empty() {
                return Err(AppError::invalid("Nome de pasta vazio nos filtros"));
            }
        }
        Ok(())
    }

    pub fn to_rules(&self) -> Vec<String> {
        let mut rules: Vec<String> = self
            .custom_rules
            .iter()
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .collect();

        if self.exclude_hidden {
            rules.extend(HIDDEN_RULES.iter().map(|s| s.to_string()));
        }

        for d in &self.exclude_dirs {
            let d = d.trim().replace('\\', "/");
            let anchored = d.starts_with('/');
            let name = escape_glob(d.trim_matches('/'));
            if name.is_empty() {
                continue;
            }
            rules.push(if anchored { format!("- /{name}/**") } else { format!("- {name}/**") });
        }

        for e in &self.exclude_exts {
            let ext = e.trim().trim_start_matches('*').trim_start_matches('.');
            if ext.is_empty() {
                continue;
            }
            let lower = escape_glob(&ext.to_lowercase());
            let upper = escape_glob(&ext.to_uppercase());
            rules.push(format!("- *.{lower}"));
            if upper != lower {
                rules.push(format!("- *.{upper}"));
            }
        }
        rules
    }
}

fn escape_glob(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_rules_in_order() {
        let f = Filters {
            exclude_dirs: vec!["node_modules".into(), "/build".into(), "a[1]".into()],
            exclude_exts: vec!["*.tmp".into(), ".Log".into()],
            exclude_hidden: true,
            custom_rules: vec!["+ importante.tmp".into()],
        };
        let r = f.to_rules();
        assert_eq!(r[0], "+ importante.tmp");
        assert!(r.contains(&"- .*".to_string()));
        assert!(r.contains(&"- node_modules/**".to_string()));
        assert!(r.contains(&"- /build/**".to_string()));
        assert!(r.contains(&"- a\\[1\\]/**".to_string()));
        assert!(r.contains(&"- *.tmp".to_string()));
        assert!(r.contains(&"- *.TMP".to_string()));
        assert!(r.contains(&"- *.log".to_string()));
        assert!(r.contains(&"- *.LOG".to_string()));
    }

    #[test]
    fn rejects_bad_custom_rule() {
        let f = Filters { custom_rules: vec!["foo".into()], ..Default::default() };
        assert!(f.validate().is_err());
    }
}
