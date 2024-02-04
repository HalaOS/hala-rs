use std::{
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

pub fn clap_parse_sockaddrs(s: &str) -> Result<Vec<SocketAddr>, String> {
    let splits = s.split(":").collect::<Vec<_>>();

    if splits.len() != 2 {
        return Err(format!(
            "Invalid address string: {}. the desired format is `ip_or_domain_name:port-range`",
            s
        ));
    }

    let mut parsed_addrs = vec![];

    for port in clap_parse_ports(splits[1])? {
        let mut addrs = (splits[0], port)
            .to_socket_addrs()
            .map_err(|err| err.to_string())?
            .collect::<Vec<_>>();

        parsed_addrs.append(&mut addrs);
    }

    Ok(parsed_addrs)
}
