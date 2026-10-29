//! TLS to the bridge with the owner's pin as the only trust.
//!
//! The bridge's certificate is its own (self-signed, ten years, made for webpki: `CA:FALSE`, `serverAuth`). The TV trusts it by the SHA-256 of
//! its DER, which the owner copied from the bridge's settings: no chain, no host name and no date is looked at, and no root store exists
//! here. A certificate with any other digest ends the handshake before the TV has sent a byte of its own, so the token cannot reach a
//! machine that is not the one the owner named. The handshake's signature is still checked (by `ring`, through rustls's webpki), so the
//! server must hold the key of the pinned certificate, not merely send it.
//!
//! `rustls` and `ring` are what the GFN engine already builds for this target. The connection is blocking (`StreamOwned`): the helper is
//! thread-based and a stills stream or the push channel is a thread that sits in a read.

use super::store::Pin;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{
    CryptoProvider, WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature,
};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, ClientConnection, DigitallySignedStruct, Error as TlsError,
    SignatureScheme, StreamOwned,
};
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

/// One TLS connection to the bridge.
pub type Stream = StreamOwned<ClientConnection, TcpStream>;

/// Why a connection could not be made, in the three ways the owner can act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectError {
    /// The certificate is not the pinned one (or its signature is not good): nothing was sent.
    Untrusted,
    /// Nobody answered at the address (refused, no route, too slow, hung up).
    Unreachable(String),
    /// Something else went wrong inside the handshake.
    Tls(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Untrusted => f.write_str("the bridge's certificate is not the pinned one"),
            Self::Unreachable(why) => write!(f, "the bridge did not answer ({why})"),
            Self::Tls(why) => write!(f, "the secure connection failed ({why})"),
        }
    }
}

/// The only judge of a server's certificate: its digest against the pin.
#[derive(Debug)]
struct PinVerifier {
    pin: Pin,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        if self.pin.matches_certificate(end_entity.as_ref()) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(TlsError::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// How the TV dials one bridge: its address and the configuration that holds its pin.
#[derive(Clone)]
pub struct Dialer {
    addr: SocketAddr,
    config: Arc<ClientConfig>,
}

impl std::fmt::Debug for Dialer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Dialer({})", self.addr)
    }
}

impl Dialer {
    pub fn new(addr: SocketAddr, pin: Pin) -> Self {
        let provider: Arc<CryptoProvider> = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = PinVerifier {
            pin,
            algorithms: provider.signature_verification_algorithms,
        };
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("ring does TLS 1.2 and 1.3")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        Self {
            addr,
            config: Arc::new(config),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Connects and finishes the handshake. The stream's reads and writes then wait up to `io_timeout` each; a caller that blocks for
    /// longer (a push channel) sets its own with [`Stream::sock`].
    pub fn connect(
        &self,
        connect_timeout: Duration,
        io_timeout: Duration,
    ) -> Result<Stream, ConnectError> {
        let mut tcp = TcpStream::connect_timeout(&self.addr, connect_timeout)
            .map_err(|e| ConnectError::Unreachable(e.kind().to_string()))?;
        let _ = tcp.set_nodelay(true);
        let _ = tcp.set_read_timeout(Some(io_timeout));
        let _ = tcp.set_write_timeout(Some(io_timeout));
        let name = ServerName::IpAddress(self.addr.ip().into());
        let mut connection = ClientConnection::new(self.config.clone(), name)
            .map_err(|e| ConnectError::Tls(e.to_string()))?;
        while connection.is_handshaking() {
            connection.complete_io(&mut tcp).map_err(|e| classify(&e))?;
        }
        Ok(StreamOwned::new(connection, tcp))
    }
}

/// What a failed handshake was.
fn classify(e: &io::Error) -> ConnectError {
    let tls = e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<TlsError>());
    match tls {
        // the pin's refusal, or a certificate or signature that webpki cannot read: either way it is not the bridge that was pinned
        Some(TlsError::InvalidCertificate(_)) => ConnectError::Untrusted,
        Some(other) => ConnectError::Tls(other.to_string()),
        None => match e.kind() {
            io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::TimedOut
            | io::ErrorKind::WouldBlock
            | io::ErrorKind::BrokenPipe => ConnectError::Unreachable(e.kind().to_string()),
            other => ConnectError::Tls(other.to_string()),
        },
    }
}
