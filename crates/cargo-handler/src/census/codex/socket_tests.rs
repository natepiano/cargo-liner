//! Native socket reads preserve peer relations.
#![expect(clippy::expect_used, reason = "tests need socket fixtures")]

use super::*;

#[test]
fn native_socket_peers_identify_both_ends_of_a_unix_stream() {
    let (_client, _server) = std::os::unix::net::UnixStream::pair().expect("socket pair");
    #[cfg(target_os = "linux")]
    let mut snapshot = SocketSnapshot::default();
    #[cfg(target_os = "linux")]
    let endpoints = snapshot
        .process_endpoints(std::process::id())
        .expect("socket endpoints");
    #[cfg(not(target_os = "linux"))]
    let endpoints =
        SocketSnapshot::process_endpoints(std::process::id()).expect("socket endpoints");
    assert!(
        endpoints.iter().any(|client| endpoints
            .iter()
            .any(|server| { server.socket == client.peer && server.peer == client.socket })),
        "{endpoints:?}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn unchanged_sockets_reuse_connected_and_no_peer_answers() {
    let (_client, _server) = std::os::unix::net::UnixStream::pair().expect("socket pair");
    let _unconnected = std::os::unix::net::UnixDatagram::unbound().expect("unconnected socket");
    let mut snapshot = SocketSnapshot::default();
    let endpoints = snapshot
        .process_endpoints(std::process::id())
        .expect("first read");
    assert!(
        snapshot
            .peers
            .values()
            .any(|peer| matches!(peer, SocketPeer::NoPeer))
    );
    assert!(endpoints.iter().all(|endpoint| endpoint.peer != 0));
    let queries = snapshot.query_count();
    assert!(queries > 0);
    snapshot
        .process_endpoints(std::process::id())
        .expect("second read");
    assert_eq!(
        snapshot.query_count(),
        queries,
        "unchanged sockets need no query"
    );
}
