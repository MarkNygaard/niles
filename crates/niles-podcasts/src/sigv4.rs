//! S3 request signing (Signature Version 4), hand-rolled: the only
//! operations Niles needs are a header-signed PUT and DELETE, and a
//! presigned GET, against one bucket. A general S3 client would carry
//! ten times this for nothing.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

fn hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

fn hmac(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(key)
        .expect("HMAC-SHA256 accepts any key length");
    mac.update(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&mac.finalize().into_bytes());
    out
}

/// AWS URI-encoding: every byte outside `A-Z a-z 0-9 - _ . ~` becomes
/// `%XX` in uppercase hex. `/` survives only when `keep_slash` — in a
/// canonical URI, not in a query value.
pub(crate) fn encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The signing key chain: `"AWS4" + secret` through `date`, `region`,
/// `service`, and `aws4_request`.
pub(crate) fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> [u8; 32] {
    let k_secret = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac(&k_secret, region.as_bytes());
    let k_service = hmac(&k_region, service.as_bytes());
    hmac(&k_service, b"aws4_request")
}

pub(crate) struct Credentials<'a> {
    pub access_key_id: &'a str,
    pub secret: &'a str,
    pub region: &'a str,
}

fn amz_date(now: DateTime<Utc>) -> (String, String) {
    (
        now.format("%Y%m%dT%H%M%SZ").to_string(),
        now.format("%Y%m%d").to_string(),
    )
}

#[allow(clippy::too_many_arguments)]
fn signature(
    creds: &Credentials,
    method: &str,
    canonical_uri: &str,
    canonical_query: &str,
    canonical_headers: &str,
    signed_headers: &str,
    payload_hash: &str,
    now: DateTime<Utc>,
) -> (String, String, String) {
    let (amz, date) = amz_date(now);
    let scope = format!("{date}/{}/s3/aws4_request", creds.region);
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz}\n{scope}\n{}",
        hex(&sha256(canonical_request.as_bytes()))
    );
    let key = signing_key(creds.secret, &date, creds.region, "s3");
    (hex(&hmac(&key, string_to_sign.as_bytes())), amz, scope)
}

/// The `Authorization` header for a header-signed request. `headers`
/// must include `host`; names are lowercased and sorted here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn authorization(
    creds: &Credentials,
    method: &str,
    _host: &str,
    canonical_uri: &str,
    canonical_query: &str,
    headers: &[(&str, &str)],
    payload_hash: &str,
    now: DateTime<Utc>,
) -> String {
    let mut sorted: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers = sorted
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect::<String>();
    let signed_headers = sorted
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let (sig, _, scope) = signature(
        creds,
        method,
        canonical_uri,
        canonical_query,
        &canonical_headers,
        &signed_headers,
        payload_hash,
        now,
    );
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={sig}",
        creds.access_key_id
    )
}

/// A presigned URL for `method`: `host` is the only signed header and
/// the payload is `UNSIGNED-PAYLOAD`, so a player fetching it may send
/// Range headers without breaking the signature. `extra` are further
/// query parameters, given unencoded and signed with the rest, such as
/// `response-content-disposition`, which S3 and R2 answer with.
#[allow(clippy::too_many_arguments)]
pub(crate) fn presign(
    creds: &Credentials,
    method: &str,
    scheme: &str,
    host: &str,
    canonical_uri: &str,
    expires_secs: u64,
    extra: &[(&str, &str)],
    now: DateTime<Utc>,
) -> String {
    let (amz, date) = amz_date(now);
    let scope = format!("{date}/{}/s3/aws4_request", creds.region);
    let credential = encode(&format!("{}/{scope}", creds.access_key_id), false);
    let mut params = vec![
        ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_string()),
        ("X-Amz-Credential", credential),
        ("X-Amz-Date", amz),
        ("X-Amz-Expires", expires_secs.to_string()),
        ("X-Amz-SignedHeaders", "host".to_string()),
    ];
    params.extend(
        extra
            .iter()
            .map(|(name, value)| (*name, encode(value, false))),
    );
    params.sort_by(|a, b| a.0.cmp(b.0));
    let query = params
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let canonical_headers = format!("host:{host}\n");
    let (sig, _, _) = signature(
        creds,
        method,
        canonical_uri,
        &query,
        &canonical_headers,
        "host",
        "UNSIGNED-PAYLOAD",
        now,
    );
    format!("{scheme}://{host}{canonical_uri}?{query}&X-Amz-Signature={sig}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    #[test]
    fn encode_keeps_only_what_aws_keeps() {
        assert_eq!(encode("a b/c~", false), "a%20b%2Fc~");
        assert_eq!(encode("/test$file.text", true), "/test%24file.text");
    }

    #[test]
    fn the_signing_key_matches_awss_iam_example() {
        // AWS's " deriving the signing key" example, service iam.
        let key = signing_key(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20120215",
            "us-east-1",
            "iam",
        );
        assert_eq!(
            hex(&key),
            "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d"
        );
    }

    #[test]
    fn an_extra_parameter_is_encoded_sorted_and_signed() {
        let creds = Credentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE",
            secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            region: "auto",
        };
        let at = utc(2026, 10, 18, 12, 0, 0);
        let sign = |extra: &[(&str, &str)]| {
            presign(
                &creds,
                "GET",
                "https",
                "r2.example",
                "/b/k.mp3",
                600,
                extra,
                at,
            )
        };
        let plain = sign(&[]);
        let download = sign(&[(
            "response-content-disposition",
            "attachment; filename=\"a b.mp3\"",
        )]);
        let query = download.split_once('?').unwrap().1;
        assert!(
            query.contains(
                "X-Amz-SignedHeaders=host&response-content-disposition=\
                 attachment%3B%20filename%3D%22a%20b.mp3%22&X-Amz-Signature="
            ),
            "after the X-Amz- names, encoded, before the signature: {query}"
        );
        let signature = |url: &str| url.rsplit_once("X-Amz-Signature=").unwrap().1.to_string();
        assert_ne!(
            signature(&plain),
            signature(&download),
            "the parameter is signed"
        );
    }

    /// The AWS SigV4 presigned-URL test vector (get object, virtual-hosted).
    #[test]
    fn the_aws_presigned_get_example() {
        let creds = Credentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE",
            secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
        };
        let url = presign(
            &creds,
            "GET",
            "https",
            "examplebucket.s3.amazonaws.com",
            "/test.txt",
            86400,
            &[],
            utc(2013, 5, 24, 0, 0, 0),
        );
        assert_eq!(
            url,
            "https://examplebucket.s3.amazonaws.com/test.txt?X-Amz-Algorithm=AWS4-HMAC-SHA256\
             &X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request\
             &X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host\
             &X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"
        );
        // The canonical request's hash, as the AWS example prints it:
        // a guard against silently changing how the request is built.
        let canonical = "GET\n/test.txt\nX-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=\
                         AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&\
                         X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host\n\
                         host:examplebucket.s3.amazonaws.com\n\nhost\nUNSIGNED-PAYLOAD";
        assert_eq!(
            hex(&sha256(canonical.as_bytes())),
            "3bfa292879f6447bbcda7001decf97f4a54dc650c8942174ae0a9121cf58ad04"
        );
    }

    /// The AWS SigV4 header-signed GET test vector (get object, range).
    #[test]
    fn the_aws_header_signed_get_example() {
        let creds = Credentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE",
            secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
        };
        let auth = authorization(
            &creds,
            "GET",
            "examplebucket.s3.amazonaws.com",
            "/test.txt",
            "",
            &[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("range", "bytes=0-9"),
                (
                    "x-amz-content-sha256",
                    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                ),
                ("x-amz-date", "20130524T000000Z"),
            ],
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            utc(2013, 5, 24, 0, 0, 0),
        );
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    /// The AWS SigV4 PUT test vector (object with storage class).
    #[test]
    fn the_aws_put_example() {
        let creds = Credentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE",
            secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
        };
        let body = "Welcome to Amazon S3.";
        let payload_hash = hex(&sha256(body.as_bytes()));
        assert_eq!(
            payload_hash,
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        let auth = authorization(
            &creds,
            "PUT",
            "examplebucket.s3.amazonaws.com",
            encode("/test$file.text", true).as_str(),
            "",
            &[
                ("date", "Fri, 24 May 2013 00:00:00 GMT"),
                ("host", "examplebucket.s3.amazonaws.com"),
                ("x-amz-content-sha256", &payload_hash),
                ("x-amz-date", "20130524T000000Z"),
                ("x-amz-storage-class", "REDUCED_REDUNDANCY"),
            ],
            &payload_hash,
            utc(2013, 5, 24, 0, 0, 0),
        );
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class, \
             Signature=98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
    }
}
