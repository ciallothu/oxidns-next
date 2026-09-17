// SPDX-FileCopyrightText: 2025 Sven Shi
// SPDX-License-Identifier: GPL-3.0-or-later

//! Normalized IP prefixes shared by RouterOS integrations.

use std::fmt::{Display, Formatter};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) struct IpPrefix {
    address: IpAddr,
    prefix: u8,
}

impl IpPrefix {
    #[cfg(feature = "plugin-ros-address-list")]
    pub(crate) fn host(address: IpAddr) -> Self {
        Self {
            address,
            prefix: host_prefix(address),
        }
    }

    pub(crate) fn new(address: IpAddr, prefix: u8) -> Option<Self> {
        (prefix <= host_prefix(address)).then(|| Self {
            address: normalize(address, prefix),
            prefix,
        })
    }

    pub(crate) fn address(self) -> IpAddr {
        self.address
    }

    pub(crate) fn prefix(self) -> u8 {
        self.prefix
    }

    #[cfg(feature = "plugin-ros-route")]
    pub(crate) fn is_host(self) -> bool {
        self.prefix == host_prefix(self.address)
    }
}

impl Display for IpPrefix {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix)
    }
}

impl FromStr for IpPrefix {
    type Err = &'static str;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let raw = raw.trim();
        let (address, prefix) = if let Some((address, prefix)) = raw.split_once('/') {
            let address = address
                .parse::<IpAddr>()
                .map_err(|_| "invalid IP address")?;
            let prefix = prefix.parse::<u8>().map_err(|_| "invalid prefix")?;
            (address, prefix)
        } else {
            let address = raw.parse::<IpAddr>().map_err(|_| "invalid IP address")?;
            (address, host_prefix(address))
        };
        Self::new(address, prefix).ok_or("prefix exceeds address width")
    }
}

pub(crate) fn host_prefix(address: IpAddr) -> u8 {
    match address {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    }
}

fn normalize(address: IpAddr, prefix: u8) -> IpAddr {
    match address {
        IpAddr::V4(address) => {
            let host_bits = 32u8.saturating_sub(prefix);
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << host_bits
            };
            IpAddr::V4(Ipv4Addr::from(u32::from(address) & mask))
        }
        IpAddr::V6(address) => {
            let host_bits = 128u8.saturating_sub(prefix);
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << host_bits
            };
            IpAddr::V6(Ipv6Addr::from(u128::from(address) & mask))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_addresses_become_hosts_and_cidrs_are_normalized() {
        let host = "192.0.2.9".parse::<IpPrefix>().expect("host");
        assert_eq!(host.to_string(), "192.0.2.9/32");
        assert_eq!(host.prefix(), 32);

        let network = "192.0.2.129/24".parse::<IpPrefix>().expect("network");
        assert_eq!(network.to_string(), "192.0.2.0/24");
        assert_eq!(network.prefix(), 24);
    }
}
