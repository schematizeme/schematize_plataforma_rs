//! Resolução do HOME do usuário, compartilhada por `parasita` e `bin`.
//!
//! **O quê:** `HOME` -> `USERPROFILE`, descartando valor vazio, em branco ou relativo.
//!
//! **Onde:** `parasita::garantir` e `bin::home`. A leitura do ambiente é injetável para os
//! testes nunca mexerem no HOME real (estado global que testes paralelos roubariam).

use std::path::{Path, PathBuf};

/// **O quê:** o HOME do usuário lido do ambiente real, ou `None` se nenhum valor serve.
///
/// **Onde:** `parasita::garantir` e `bin::home`.
pub fn home_do_ambiente() -> Option<PathBuf> {
    home_de(&|k| std::env::var_os(k).map(|v| v.to_string_lossy().into_owned()))
}

/// **O quê:** a cadeia `HOME -> USERPROFILE` sobre um leitor injetado. Valor vazio, só com
/// espaços ou RELATIVO é descartado e cai para o próximo (HOME relativo escreveria no cwd).
/// Unicode e espaços no meio são válidos.
///
/// **Onde:** [`home_do_ambiente`] e os testes com HOMEs hostis.
pub fn home_de(get: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .filter_map(|k| get(k))
        .find(|v| !v.trim().is_empty() && Path::new(v).is_absolute())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **O quê:** leitor de ambiente falso a partir de pares.
    ///
    /// **Onde:** os testes deste módulo.
    fn amb(pares: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pares.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    /// **O quê:** HOME tem precedência; USERPROFILE é o fallback.
    ///
    /// **Onde:** teste de `home_de`.
    #[test]
    fn home_vence_e_userprofile_e_fallback() {
        let a = amb(&[("HOME", "/h"), ("USERPROFILE", "/u")]);
        assert_eq!(home_de(&a), Some(PathBuf::from("/h")));
        let b = amb(&[("USERPROFILE", "/u")]);
        assert_eq!(home_de(&b), Some(PathBuf::from("/u")));
        assert_eq!(home_de(&amb(&[])), None);
    }

    /// **O quê:** valores hostis (vazio, branco, relativo) caem para o próximo ou `None`.
    ///
    /// **Onde:** teste de `home_de`.
    #[test]
    fn home_hostil_vazio_branco_relativo() {
        assert_eq!(home_de(&amb(&[("HOME", ""), ("USERPROFILE", "/u")])), Some("/u".into()));
        assert_eq!(home_de(&amb(&[("HOME", "   "), ("USERPROFILE", "/u")])), Some("/u".into()));
        assert_eq!(home_de(&amb(&[("HOME", "."), ("USERPROFILE", "/u")])), Some("/u".into()));
        assert_eq!(home_de(&amb(&[("HOME", "rel/dir")])), None);
        assert_eq!(home_de(&amb(&[("HOME", "")])), None);
    }

    /// **O quê:** unicode e espaços no caminho são preservados byte a byte.
    ///
    /// **Onde:** teste de `home_de`.
    #[test]
    fn home_unicode_e_espacos_sao_preservados() {
        let a = amb(&[("HOME", "/home/josé da silva/日本語 ünï")]);
        assert_eq!(home_de(&a), Some(PathBuf::from("/home/josé da silva/日本語 ünï")));
        let b = amb(&[("HOME", " /com espaço ")]);
        // Começa com espaço: é relativo (não absoluto), logo descartado.
        assert_eq!(home_de(&b), None);
    }

    /// **O quê:** o leitor real não entra em pânico (HOME do teste é o do processo).
    ///
    /// **Onde:** teste de `home_do_ambiente`.
    #[test]
    fn leitura_real_nao_entra_em_panico() {
        let _ = home_do_ambiente();
    }
}
