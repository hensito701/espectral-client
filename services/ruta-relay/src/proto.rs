//! Header line parsing (`ERT1 <grant> <dest-ip> <dest-port>\n`).
//!
//! Rules from `PROTOCOL.md`:
//! - whole line ≤ 4096 bytes including `\n`, else `ERR proto`;
//! - one ASCII line, exactly four fields separated by *single* spaces;
//! - `<dest-ip>` is an IP literal (no brackets, no DNS);
//! - `<dest-port>` is decimal 1–65535.
//!
//! Anything that does not match is `ERR proto`; the destination policy is a
//! separate check (`dest::allowed_with`), so a well-formed line naming a
//! private address is still `proto`-clean and fails later with `ERR dest`.

use std::net::{IpAddr, SocketAddr};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::Code;

/// Hard cap from the protocol: the whole line including `\n`.
pub const MAX_HEADER_LINE: usize = 4096;

/// A parsed handshake line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub grant: String,
    pub dest: SocketAddr,
}

/// Reads one handshake line, returning the request plus any bytes that were
/// read past the `\n` (a client may pipeline the Minecraft handshake right
/// behind the relay header; those bytes must be forwarded upstream verbatim).
pub async fn read_header<R: AsyncRead + Unpin>(r: &mut R) -> Result<(Request, Vec<u8>), Code> {
    let mut buf = vec![0u8; MAX_HEADER_LINE];
    let mut len = 0usize;
    loop {
        if len == MAX_HEADER_LINE {
            // No `\n` within the cap.
            return Err(Code::Proto);
        }
        let n = r.read(&mut buf[len..]).await.map_err(|_| Code::Proto)?;
        if n == 0 {
            return Err(Code::Proto);
        }
        len += n;
        if let Some(pos) = buf[..len].iter().position(|&b| b == b'\n') {
            let rest = buf[pos + 1..len].to_vec();
            let req = parse_header(&buf[..pos])?;
            return Ok((req, rest));
        }
    }
}

/// Parses a header line without its trailing `\n`.
pub fn parse_header(line: &[u8]) -> Result<Request, Code> {
    if line.is_empty() || line.len() >= MAX_HEADER_LINE {
        return Err(Code::Proto);
    }
    if !line.is_ascii() {
        return Err(Code::Proto);
    }
    let s = std::str::from_utf8(line).map_err(|_| Code::Proto)?;
    let mut fields = s.split(' ');
    let version = fields.next().ok_or(Code::Proto)?;
    if version != "ERT1" {
        return Err(Code::Proto);
    }
    let grant = fields.next().ok_or(Code::Proto)?;
    let ip = fields.next().ok_or(Code::Proto)?;
    let port = fields.next().ok_or(Code::Proto)?;
    if fields.next().is_some() {
        return Err(Code::Proto);
    }
    // Exactly one single space between fields: empty fields can only come from
    // doubled, leading or trailing spaces.
    if grant.is_empty() || ip.is_empty() || port.is_empty() {
        return Err(Code::Proto);
    }

    let ip: IpAddr = ip.parse().map_err(|_| Code::Proto)?;
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Code::Proto);
    }
    let port: u16 = port.parse().map_err(|_| Code::Proto)?;
    if port == 0 {
        return Err(Code::Proto);
    }

    Ok(Request {
        grant: grant.to_owned(),
        dest: SocketAddr::new(ip, port),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    const G: &str = "a.b.c";

    #[test]
    fn accepts_ipv4_and_ipv6() {
        let r = parse_header(&line("ERT1 a.b.c 1.2.3.4 25565")).unwrap();
        assert_eq!(r.grant, G);
        assert_eq!(r.dest, "1.2.3.4:25565".parse().unwrap());

        let r = parse_header(&line("ERT1 a.b.c 2606:4700:4700::1111 25565")).unwrap();
        assert_eq!(r.dest, "[2606:4700:4700::1111]:25565".parse().unwrap());

        let r = parse_header(&line("ERT1 a.b.c 0.0.0.0 1024")).unwrap();
        assert_eq!(r.dest.port(), 1024);
    }

    #[test]
    fn rejects_missing_fields() {
        for s in [
            "",
            "ERT1",
            "ERT1 a.b.c",
            "ERT1 a.b.c 1.2.3.4",
            "ERT1 a.b.c 1.2.3.4 25565 extra",
        ] {
            assert_eq!(parse_header(&line(s)), Err(Code::Proto), "{s:?}");
        }
    }

    #[test]
    fn rejects_crlf() {
        // The line ends at `\n`; a `\r` stays part of the port field and is not
        // a decimal digit sequence.
        assert_eq!(
            parse_header(&line("ERT1 a.b.c 1.2.3.4 25565\r")),
            Err(Code::Proto)
        );
    }

    #[test]
    fn rejects_extra_spaces() {
        for s in [
            "ERT1  a.b.c 1.2.3.4 25565",
            "ERT1 a.b.c  1.2.3.4 25565",
            "ERT1 a.b.c 1.2.3.4  25565",
            "ERT1 a.b.c 1.2.3.4 25565 ",
            " ERT1 a.b.c 1.2.3.4 25565",
            "ERT1\ta.b.c\t1.2.3.4\t25565",
        ] {
            assert_eq!(parse_header(&line(s)), Err(Code::Proto), "{s:?}");
        }
    }

    #[test]
    fn rejects_bad_version_and_fields() {
        for s in [
            "ERT2 a.b.c 1.2.3.4 25565",
            "ert1 a.b.c 1.2.3.4 25565",
            "ERT1 a.b.c not-an-ip 25565",
            "ERT1 a.b.c [::1] 25565",
            "ERT1 a.b.c 1.2.3.4 abc",
            "ERT1 a.b.c 1.2.3.4 -1",
            "ERT1 a.b.c 1.2.3.4 +25565",
            "ERT1 a.b.c 1.2.3.4 65536",
            "ERT1 a.b.c 1.2.3.4 0",
            "ERT1 a.b.c 1.2.3.4 0x1f90",
            "ERT1  a.b.c 1.2.3.4 25565",
        ] {
            assert_eq!(parse_header(&line(s)), Err(Code::Proto), "{s:?}");
        }
    }

    #[test]
    fn rejects_non_ascii() {
        let mut v = line("ERT1 a.b.c 1.2.3.4 25565");
        v[5] = 0xc3;
        assert_eq!(parse_header(&v), Err(Code::Proto));
    }

    #[tokio::test]
    async fn reader_splits_pipelined_bytes() {
        let input = b"ERT1 a.b.c 1.2.3.4 25565\n\x10\x00extra".to_vec();
        let mut cur = std::io::Cursor::new(input);
        let (req, rest) = read_header(&mut cur).await.unwrap();
        assert_eq!(req.dest, "1.2.3.4:25565".parse().unwrap());
        assert_eq!(rest, b"\x10\x00extra");
    }

    #[tokio::test]
    async fn reader_accepts_max_size_line() {
        // "ERT1 " + grant + " 1.2.3.4 25565\n" must total exactly 4096 bytes.
        let pad = 4096 - ("ERT1 ".len() + " 1.2.3.4 25565\n".len());
        let mut input = b"ERT1 ".to_vec();
        input.extend(std::iter::repeat(b'a').take(pad));
        input.extend_from_slice(b" 1.2.3.4 25565\n");
        assert_eq!(input.len(), 4096);
        let mut cur = std::io::Cursor::new(input);
        let (req, rest) = read_header(&mut cur).await.unwrap();
        assert_eq!(req.grant.len(), pad);
        assert!(rest.is_empty());
    }

    #[tokio::test]
    async fn reader_rejects_oversize() {
        let mut input = b"ERT1 ".to_vec();
        input.extend(std::iter::repeat(b'a').take(4096));
        input.extend_from_slice(b" 1.2.3.4 25565\n");
        let mut cur = std::io::Cursor::new(input);
        assert_eq!(read_header(&mut cur).await.unwrap_err(), Code::Proto);
    }

    #[tokio::test]
    async fn reader_rejects_eof_without_newline() {
        let mut cur = std::io::Cursor::new(b"ERT1 a.b.c 1.2.3.4 25565".to_vec());
        assert_eq!(read_header(&mut cur).await.unwrap_err(), Code::Proto);
    }

    #[tokio::test]
    async fn reader_rejects_crlf_through_socket() {
        let mut cur = std::io::Cursor::new(b"ERT1 a.b.c 1.2.3.4 25565\r\n".to_vec());
        assert_eq!(read_header(&mut cur).await.unwrap_err(), Code::Proto);
    }
}
