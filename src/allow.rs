//! The listener accepts one Tailscale peer and refuses every other address.

use std::net::IpAddr;

pub fn peer_allowed(peer: IpAddr, allowed: &[IpAddr]) -> bool {
    allowed.iter().any(|candidate| candidate == &peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn only_the_windows_peer_is_accepted() {
        let allowed = [IpAddr::V4(Ipv4Addr::new(100, 84, 4, 24))];
        assert!(peer_allowed(allowed[0], &allowed));
        assert!(!peer_allowed(
            IpAddr::V4(Ipv4Addr::new(100, 96, 187, 43)),
            &allowed
        ));
        assert!(!peer_allowed(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)), &allowed));
    }
}
