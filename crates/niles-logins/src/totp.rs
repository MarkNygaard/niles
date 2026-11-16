//! A saved login's authenticator key, and the codes Niles makes from it
//! (RFC 6238), so a site that asks for an authenticator app's code
//! needs no card.
//!
//! A key comes as a site shows it: the setup key (base32), or the
//! `otpauth://totp/...` link behind the QR code. It is kept in one
//! canonical form, checked before anything is kept, and no error quotes
//! what was given.

use crate::error::{Error, Result};
use crate::model::Secret;
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};

/// The shortest key accepted: 80 bits, what most sites hand out.
const MIN_KEY_BYTES: usize = 10;
/// The longest given text accepted.
pub const MAX_AUTHENTICATOR: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    fn name(self) -> &'static str {
        match self {
            Algorithm::Sha1 => "SHA1",
            Algorithm::Sha256 => "SHA256",
            Algorithm::Sha512 => "SHA512",
        }
    }
}

/// A key ready to make codes. Never printed: its `Debug` shows nothing
/// of the key.
pub struct Authenticator {
    key: Vec<u8>,
    digits: u32,
    period: u64,
    algorithm: Algorithm,
}

impl std::fmt::Debug for Authenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Authenticator(…)")
    }
}

fn invalid(reason: &str) -> Error {
    Error::Invalid {
        kind: "authenticator key",
        reason: reason.to_string(),
    }
}

impl Authenticator {
    /// Reads a key as a person pastes it, or as [`authenticator`] keeps it.
    pub fn parse(given: &str) -> Result<Authenticator> {
        let given = given.trim();
        if given.is_empty() {
            return Err(invalid("it is empty"));
        }
        if given.chars().count() > MAX_AUTHENTICATOR {
            return Err(invalid("it is longer than any authenticator key"));
        }
        if given.len() >= 10 && given[..10].eq_ignore_ascii_case("otpauth://") {
            return Self::from_link(given);
        }
        Ok(Authenticator {
            key: base32(given)?,
            digits: 6,
            period: 30,
            algorithm: Algorithm::Sha1,
        })
    }

    fn from_link(link: &str) -> Result<Authenticator> {
        let url = url::Url::parse(link).map_err(|_| invalid("the otpauth link doesn't read"))?;
        if !url
            .host_str()
            .is_some_and(|h| h.eq_ignore_ascii_case("totp"))
        {
            return Err(invalid(
                "only time-based codes (otpauth://totp) are supported",
            ));
        }
        let mut secret = None;
        let mut digits = 6;
        let mut period = 30;
        let mut algorithm = Algorithm::Sha1;
        for (name, value) in url.query_pairs() {
            match name.to_ascii_lowercase().as_str() {
                "secret" => secret = Some(value.to_string()),
                "digits" => {
                    digits = match value.as_ref() {
                        "6" => 6,
                        "8" => 8,
                        _ => return Err(invalid("codes must be 6 or 8 digits")),
                    }
                }
                "period" => {
                    period = value
                        .parse::<u64>()
                        .ok()
                        .filter(|p| (1..=300).contains(p))
                        .ok_or_else(|| invalid("the period must be 1 to 300 seconds"))?;
                }
                "algorithm" => {
                    algorithm = match value.to_ascii_uppercase().as_str() {
                        "SHA1" => Algorithm::Sha1,
                        "SHA256" => Algorithm::Sha256,
                        "SHA512" => Algorithm::Sha512,
                        _ => return Err(invalid("the algorithm must be SHA1, SHA256 or SHA512")),
                    }
                }
                _ => {}
            }
        }
        let secret = secret.ok_or_else(|| invalid("the otpauth link has no secret"))?;
        Ok(Authenticator {
            key: base32(&secret)?,
            digits,
            period,
            algorithm,
        })
    }

    /// The key as it is kept: one link with every setting spelled out.
    pub fn canonical(&self) -> Secret {
        Secret::new(format!(
            "otpauth://totp/?secret={}&digits={}&period={}&algorithm={}",
            data_encoding::BASE32_NOPAD.encode(&self.key),
            self.digits,
            self.period,
            self.algorithm.name()
        ))
    }

    /// The code an authenticator app shows at `at`.
    pub fn code_at(&self, at: DateTime<Utc>) -> Secret {
        let step = u64::try_from(at.timestamp()).unwrap_or(0) / self.period;
        let message = step.to_be_bytes();
        let mac = match self.algorithm {
            Algorithm::Sha1 => {
                let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&self.key).expect(ANY_KEY);
                mac.update(&message);
                mac.finalize().into_bytes().to_vec()
            }
            Algorithm::Sha256 => {
                let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&self.key).expect(ANY_KEY);
                mac.update(&message);
                mac.finalize().into_bytes().to_vec()
            }
            Algorithm::Sha512 => {
                let mut mac = Hmac::<sha2::Sha512>::new_from_slice(&self.key).expect(ANY_KEY);
                mac.update(&message);
                mac.finalize().into_bytes().to_vec()
            }
        };
        // RFC 4226's dynamic truncation.
        let offset = usize::from(mac[mac.len() - 1] & 0x0f);
        let number = u32::from_be_bytes([
            mac[offset] & 0x7f,
            mac[offset + 1],
            mac[offset + 2],
            mac[offset + 3],
        ]);
        let code = number % 10u32.pow(self.digits);
        Secret::new(format!("{code:0width$}", width = self.digits as usize))
    }
}

/// Why `new_from_slice` can't fail here.
const ANY_KEY: &str = "HMAC takes a key of any length";

/// A base32 key as sites write it: any case, in groups split by spaces or
/// dashes, padded or not.
fn base32(given: &str) -> Result<Vec<u8>> {
    let cleaned: String = given
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let cleaned = cleaned.trim_end_matches('=');
    let key = data_encoding::BASE32_NOPAD
        .decode(cleaned.as_bytes())
        .map_err(|_| invalid("it isn't a base32 setup key (letters A-Z and digits 2-7)"))?;
    if key.len() < MIN_KEY_BYTES {
        return Err(invalid("it is shorter than any authenticator key"));
    }
    Ok(key)
}

/// A pasted key, checked and put in its kept form.
pub fn authenticator(given: Secret) -> Result<Secret> {
    Ok(Authenticator::parse(given.expose())?.canonical())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0)
            .single()
            .expect("a valid time")
    }

    fn rfc(key: &[u8], algorithm: &str) -> Authenticator {
        let link = format!(
            "otpauth://totp/Example?secret={}&digits=8&algorithm={algorithm}",
            data_encoding::BASE32_NOPAD.encode(key)
        );
        Authenticator::parse(&link).expect("a valid link")
    }

    #[test]
    fn rfc_6238s_test_vectors_hold() {
        let sha1 = rfc(b"12345678901234567890", "SHA1");
        let sha256 = rfc(b"12345678901234567890123456789012", "SHA256");
        let sha512 = rfc(
            b"1234567890123456789012345678901234567890123456789012345678901234",
            "SHA512",
        );
        for (seconds, one, two, five) in [
            (59, "94287082", "46119246", "90693936"),
            (1_111_111_109, "07081804", "68084774", "25091201"),
            (1_111_111_111, "14050471", "67062674", "99943326"),
            (1_234_567_890, "89005924", "91819424", "93441116"),
            (2_000_000_000, "69279037", "90698825", "38618901"),
            (20_000_000_000, "65353130", "77737706", "47863826"),
        ] {
            assert_eq!(sha1.code_at(at(seconds)).expose(), one, "SHA1 at {seconds}");
            assert_eq!(
                sha256.code_at(at(seconds)).expose(),
                two,
                "SHA256 at {seconds}"
            );
            assert_eq!(
                sha512.code_at(at(seconds)).expose(),
                five,
                "SHA512 at {seconds}"
            );
        }
    }

    #[test]
    fn a_setup_key_reads_however_it_was_written() {
        // "JBSWY3DPEHPK3PXP" is the usual example key.
        let plain = Authenticator::parse("JBSWY3DPEHPK3PXP").expect("plain");
        let grouped = Authenticator::parse("jbsw y3dp-ehpk 3pxp").expect("grouped");
        assert_eq!(plain.canonical(), grouped.canonical());
        assert_eq!(plain.code_at(at(59)).expose().len(), 6);
        assert_eq!(plain.code_at(at(59)), grouped.code_at(at(59)));
    }

    #[test]
    fn the_kept_form_makes_the_same_codes() {
        let given = "otpauth://totp/Shop:mark?secret=JBSWY3DPEHPK3PXP&issuer=Shop&period=60";
        let kept = authenticator(Secret::new(given)).expect("valid");
        let again = Authenticator::parse(kept.expose()).expect("the kept form reads");
        let first = Authenticator::parse(given).expect("valid");
        for seconds in [0, 59, 61, 1_000_000] {
            assert_eq!(first.code_at(at(seconds)), again.code_at(at(seconds)));
        }
    }

    #[test]
    fn a_bad_key_is_refused_without_quoting_it() {
        for bad in [
            "",
            "not base32 at all!",
            "JBSW",
            "otpauth://hotp/Shop?secret=JBSWY3DPEHPK3PXP&counter=1",
            "otpauth://totp/Shop?issuer=Shop",
            "otpauth://totp/Shop?secret=JBSWY3DPEHPK3PXP&digits=7",
            "otpauth://totp/Shop?secret=JBSWY3DPEHPK3PXP&algorithm=MD5",
        ] {
            let error = authenticator(Secret::new(bad)).expect_err(bad).to_string();
            if !bad.is_empty() {
                assert!(!error.contains(bad), "{error}");
            }
        }
    }

    #[test]
    fn nothing_prints_the_key() {
        let key = Authenticator::parse("JBSWY3DPEHPK3PXP").expect("valid");
        assert!(!format!("{key:?}").contains("JBSW"));
    }
}
