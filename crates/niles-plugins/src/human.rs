//! Network failures as a person reads them.
//!
//! What rustls, rumqttc, reqwest and the OS say is written for whoever
//! debugs them: "peer closed connection without sending TLS
//! close_notify: https://docs.rs/rustls/…", "No connection could be
//! made because the target machine actively refused it. (os error
//! 10061)". A plugin's card, its status line and what it tells the
//! household get a few plain words instead; the raw text goes to the
//! plugin's log at debug level.

use std::error::Error;
use std::io::ErrorKind;

/// Why it failed, in a few words, from `e` and what caused it. `peer`
/// is who was on the other end: "the broker", "the server".
pub fn said(e: &(dyn Error + 'static), peer: &str) -> String {
    reason(e, peer).unwrap_or_else(|| plain(&e.to_string()))
}

/// The first error in the chain Niles knows how to say, said.
fn reason(e: &(dyn Error + 'static), peer: &str) -> Option<String> {
    let mut next = Some(e);
    while let Some(e) = next {
        if let Some(tls) = e.downcast_ref::<rustls::Error>() {
            return Some(tls_reason(tls, peer));
        }
        if let Some(io) = e.downcast_ref::<std::io::Error>() {
            // rustls hands its errors back inside an io::Error, which
            // `source` does not always reach.
            if let Some(inner) = io.get_ref()
                && let Some(said) = reason(inner, peer)
            {
                return Some(said);
            }
            if let Some(said) = io_reason(io.kind(), peer) {
                return Some(said);
            }
        }
        next = e.source();
    }
    None
}

fn io_reason(kind: ErrorKind, peer: &str) -> Option<String> {
    Some(match kind {
        ErrorKind::ConnectionRefused => "connection refused".into(),
        ErrorKind::TimedOut => "timed out".into(),
        ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::BrokenPipe
        | ErrorKind::UnexpectedEof
        | ErrorKind::NotConnected => format!("{peer} closed the connection"),
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => {
            "the address cannot be reached from here".into()
        }
        ErrorKind::NetworkDown => "the network is down".into(),
        ErrorKind::AddrNotAvailable => "the address is not available".into(),
        _ => return None,
    })
}

fn tls_reason(e: &rustls::Error, peer: &str) -> String {
    use rustls::{CertificateError, Error};
    match e {
        Error::InvalidCertificate(c) => {
            let why = match c {
                CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
                    "it has expired"
                }
                CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                    "it is not valid yet"
                }
                CertificateError::NotValidForName
                | CertificateError::NotValidForNameContext { .. } => {
                    "it is for another address"
                }
                CertificateError::UnknownIssuer => "it is not signed by an authority Niles trusts",
                CertificateError::Revoked => "it has been revoked",
                CertificateError::BadSignature => "its signature does not check out",
                _ => "it is not valid",
            };
            format!("certificate problem: {why}")
        }
        Error::AlertReceived(_) => format!("{peer} refused the secure connection"),
        Error::NoCertificatesPresented => format!("{peer} sent no certificate"),
        Error::InappropriateMessage { .. }
        | Error::InappropriateHandshakeMessage { .. }
        | Error::InvalidMessage(_) => {
            format!("{peer} did not answer in TLS; is it the port without TLS?")
        }
        Error::PeerIncompatible(_) => format!("{peer} and Niles have no TLS version in common"),
        _ => "the secure connection failed".into(),
    }
}

/// Text Niles does not know, without the parts written for a debugger:
/// links to documentation and OS error numbers.
fn plain(text: &str) -> String {
    let mut text = text;
    for cut in [": https://", ": http://", " (os error"] {
        if let Some(at) = text.find(cut) {
            text = &text[..at];
        }
    }
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn io_error(kind: ErrorKind, text: &str) -> io::Error {
        io::Error::new(kind, text.to_string())
    }

    #[test]
    fn the_common_socket_failures_are_said_plainly() {
        let refused = io_error(
            ErrorKind::ConnectionRefused,
            "No connection could be made because the target machine actively refused it. (os error 10061)",
        );
        assert_eq!(said(&refused, "the broker"), "connection refused");
        assert_eq!(
            said(&io_error(ErrorKind::TimedOut, "x"), "the broker"),
            "timed out"
        );
        assert_eq!(
            said(&io_error(ErrorKind::ConnectionReset, "x"), "the broker"),
            "the broker closed the connection"
        );
    }

    #[test]
    fn a_tls_close_without_notify_is_the_other_end_closing() {
        let raw = io_error(
            ErrorKind::UnexpectedEof,
            "peer closed connection without sending TLS close_notify: https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof",
        );
        assert_eq!(said(&raw, "the broker"), "the broker closed the connection");
    }

    #[test]
    fn certificate_errors_say_what_is_wrong_with_it() {
        let wrapped = io::Error::new(
            ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::Expired),
        );
        assert_eq!(
            said(&wrapped, "the broker"),
            "certificate problem: it has expired"
        );
        let unknown = rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer);
        assert!(said(&unknown, "the server").starts_with("certificate problem"));
        let alert = rustls::Error::AlertReceived(rustls::AlertDescription::HandshakeFailure);
        assert_eq!(
            said(&alert, "the broker"),
            "the broker refused the secure connection"
        );
    }

    #[test]
    fn an_mqtt_io_error_is_reached_through_its_source() {
        let e = rumqttc::mqttbytes::Error::Io(io_error(ErrorKind::ConnectionAborted, "x"));
        assert_eq!(said(&e, "the broker"), "the broker closed the connection");
    }

    #[test]
    fn what_is_not_known_loses_its_links_and_error_numbers() {
        let e = io_error(
            ErrorKind::Other,
            "something odd happened (os error 1234)",
        );
        assert_eq!(said(&e, "the broker"), "something odd happened");
        assert_eq!(
            plain("bad thing: https://docs.rs/x"),
            "bad thing"
        );
    }
}
