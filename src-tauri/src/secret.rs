use std::fmt::Write;
use zeroize::{Zeroize, Zeroizing};

/// Gera um token aleatório (CSPRNG do SO) codificado em hexadecimal.
pub fn random_token(bytes: usize) -> Zeroizing<String> {
    let mut buf = Zeroizing::new(vec![0u8; bytes]);
    getrandom::fill(&mut buf).expect("CSPRNG do sistema indisponível");
    let mut out = Zeroizing::new(String::with_capacity(bytes * 2));
    for b in buf.iter() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Senha mantida apenas em memória e apagada ao sair de escopo.
#[derive(Clone, Default)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(s: String) -> Self {
        Secret(Zeroizing::new(s))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Zeroizing<String>> for Secret {
    fn from(z: Zeroizing<String>) -> Self {
        Secret(z)
    }
}

impl PartialEq for Secret {
    /// Comparação em tempo constante (em relação ao conteúdo) para não vazar prefixos.
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.0.as_bytes(), other.0.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl<'de> serde::Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut s = String::deserialize(d)?;
        let secret = Secret::new(s.clone());
        s.zeroize();
        Ok(secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_hex_and_unique() {
        let a = random_token(32);
        let b = random_token(32);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(*a, *b);
    }
}
