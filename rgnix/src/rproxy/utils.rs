use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    ops::Range,
    time::Duration,
};

/// parse
pub fn clap_parse_ports(s: &str) -> Result<Range<u16>, String> {
    let splites = s.split("-");

    let splites = splites.collect::<Vec<_>>();

    if splites.len() == 2 {
        return Ok(Range {
            start: splites[0].parse().map_err(|err| format!("{}", err))?,
            end: splites[1].parse().map_err(|err| format!("{}", err))?,
        });
    } else if splites.len() == 1 {
        let start = splites[0].parse().map_err(|err| format!("{}", err))?;
        return Ok(Range {
            start,
            end: start + 1,
        });
    } else {
        return Err(format!(
            "Invalid port-range arg, the desired format is `a-b` or `a`"
        ));
    }
}

pub fn clap_parse_duration(s: &str) -> Result<Duration, String> {
    let duration = duration_str::parse(s).map_err(|err| format!("{}", err))?;

    Ok(duration)
}

pub fn parse_raddrs(peer_domain: &str, port_ranges: Range<u16>) -> io::Result<Vec<SocketAddr>> {
    let mut raddrs = vec![];
    for port in port_ranges {
        let mut addrs = (peer_domain, port).to_socket_addrs()?.collect::<Vec<_>>();
        raddrs.append(&mut addrs);
    }

    Ok(raddrs)
}
