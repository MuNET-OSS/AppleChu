use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use once_cell::sync::OnceCell;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    ICMP_ECHO_REPLY, IF_OPER_STATUS_OPERATIONAL, IF_TYPE_ETHERNET_CSMACD, IP_ADAPTER_ADDRESSES_LH,
    IP_ADAPTER_ADDRESSES_LH_0, IP_ADAPTER_ADDRESSES_LH_0_0, IP_ADAPTER_ADDRESSES_LH_1,
    IP_ADAPTER_DHCP_ENABLED, IP_ADAPTER_DNS_SERVER_ADDRESS_XP, IP_ADAPTER_DNS_SERVER_ADDRESS_XP_0,
    IP_ADAPTER_DNS_SERVER_ADDRESS_XP_0_0, IP_ADAPTER_GATEWAY_ADDRESS_LH,
    IP_ADAPTER_GATEWAY_ADDRESS_LH_0, IP_ADAPTER_GATEWAY_ADDRESS_LH_0_0, IP_ADAPTER_INFO,
    IP_ADAPTER_IPV4_ENABLED, IP_ADAPTER_PREFIX_XP, IP_ADAPTER_PREFIX_XP_0,
    IP_ADAPTER_PREFIX_XP_0_0, IP_ADAPTER_UNICAST_ADDRESS_LH, IP_ADAPTER_UNICAST_ADDRESS_LH_0,
    IP_ADAPTER_UNICAST_ADDRESS_LH_0_0, IP_ADDR_STRING, IP_BUF_TOO_SMALL, IP_SUCCESS, MIB_IFROW,
    MIB_IFTABLE, MIB_IF_TYPE_ETHERNET, MIB_IPFORWARDROW, MIB_IPFORWARDROW_0, MIB_IPFORWARDROW_1,
    MIB_IPROUTE_TYPE_INDIRECT,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::Networking::WinSock::{
    IpDadStatePreferred, IpPrefixOriginDhcp, IpSuffixOriginDhcp, MIB_IPPROTO_NETMGMT, SOCKADDR,
    SOCKADDR_IN, SOCKET_ADDRESS,
};
use windows_sys::Win32::System::Threading::SetEvent;

use crate::nusec::{parse_ipv4, subnet_from_config};
use applechu::amdaemon::{KeychipConfig, NetEnvConfig};
use applechu::config::Config;
use applechu::iohook::hook_table::{hook_table_apply, null_module, HookSymbol};
use applechu::iohook::proc_addr;
use applechu::util::api::Api;

const ERROR_BUFFER_OVERFLOW: u32 = 111;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_IO_PENDING: u32 = 997;
const ERROR_NOT_SUPPORTED: u32 = 50;
const AF_INET: u16 = 2;
const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;

type GetAdaptersAddressesFn =
    unsafe extern "system" fn(u32, u32, *mut c_void, *mut IP_ADAPTER_ADDRESSES_LH, *mut u32) -> u32;
type GetAdaptersInfoFn = unsafe extern "system" fn(*mut IP_ADAPTER_INFO, *mut u32) -> u32;
type GetBestRouteFn = unsafe extern "system" fn(u32, u32, *mut MIB_IPFORWARDROW) -> u32;
type GetIfTableFn = unsafe extern "system" fn(*mut MIB_IFTABLE, *mut u32, i32) -> u32;
type IcmpSendEcho2Fn = unsafe extern "system" fn(
    HANDLE,
    HANDLE,
    *mut c_void,
    *mut c_void,
    u32,
    *mut c_void,
    u16,
    *mut c_void,
    *mut c_void,
    u32,
    u32,
) -> u32;
type SendToFn = unsafe extern "system" fn(usize, *const i8, i32, i32, *const SOCKADDR, i32) -> i32;

#[derive(Clone)]
struct Network {
    subnet: u32,
    broadcast: u32,
    interface: u32,
    router: u32,
    mac: [u8; 6],
}

static NETWORK: OnceCell<Network> = OnceCell::new();
static ORIG_GET_ADAPTERS_ADDRESSES: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_ADAPTERS_INFO: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_BEST_ROUTE: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_IF_TABLE: AtomicUsize = AtomicUsize::new(0);
static ORIG_ICMP_SEND_ECHO_2: AtomicUsize = AtomicUsize::new(0);
static ORIG_SEND_TO: AtomicUsize = AtomicUsize::new(0);

static mut ORIG_GET_ADAPTERS_ADDRESSES_PTR: *const () = ptr::null();
static mut ORIG_GET_ADAPTERS_INFO_PTR: *const () = ptr::null();
static mut ORIG_GET_BEST_ROUTE_PTR: *const () = ptr::null();
static mut ORIG_GET_IF_TABLE_PTR: *const () = ptr::null();
static mut ORIG_ICMP_SEND_ECHO_2_PTR: *const () = ptr::null();
static mut ORIG_SEND_TO_PTR: *const () = ptr::null();

#[applechu_macros::config_section(stage = Platform, order = 60)]
pub fn init(api: &Api, config: &Config, section: &NetEnvConfig) {
    let Some(keychip) = config.section::<KeychipConfig>() else {
        return;
    };
    let subnet = subnet_from_config(&keychip);
    let network = Network {
        subnet,
        broadcast: parse_ipv4(&section.broadcast).unwrap_or(0xFFFF_FFFF),
        interface: subnet | (section.addr_suffix & 0xFF),
        router: subnet | (section.router_suffix & 0xFF),
        mac: parse_mac(&section.mac_addr).unwrap_or([1, 2, 3, 4, 5, 6]),
    };
    let _ = NETWORK.set(network.clone());

    unsafe {
        let iphlpapi = [
            HookSymbol {
                name: "GetAdaptersAddresses",
                patch: hooked_get_adapters_addresses as *const (),
                original: ptr::addr_of_mut!(ORIG_GET_ADAPTERS_ADDRESSES_PTR),
            },
            HookSymbol {
                name: "GetAdaptersInfo",
                patch: hooked_get_adapters_info as *const (),
                original: ptr::addr_of_mut!(ORIG_GET_ADAPTERS_INFO_PTR),
            },
            HookSymbol {
                name: "GetBestRoute",
                patch: hooked_get_best_route as *const (),
                original: ptr::addr_of_mut!(ORIG_GET_BEST_ROUTE_PTR),
            },
            HookSymbol {
                name: "GetIfTable",
                patch: hooked_get_if_table as *const (),
                original: ptr::addr_of_mut!(ORIG_GET_IF_TABLE_PTR),
            },
            HookSymbol {
                name: "IcmpSendEcho2",
                patch: hooked_icmp_send_echo_2 as *const (),
                original: ptr::addr_of_mut!(ORIG_ICMP_SEND_ECHO_2_PTR),
            },
        ];
        let ws2 = [HookSymbol {
            name: "sendto",
            patch: hooked_send_to as *const (),
            original: ptr::addr_of_mut!(ORIG_SEND_TO_PTR),
        }];
        proc_addr::push("iphlpapi.dll", &iphlpapi, sync_originals);
        proc_addr::push("ws2_32.dll", &ws2, sync_originals);
        let patched = hook_table_apply(null_module(), "iphlpapi.dll", &iphlpapi)
            + hook_table_apply(null_module(), "ws2_32.dll", &ws2);
        sync_originals();
        api.log_info(&format!(
            "LAN emulation enabled: IP {}.{}.{}.{}, gateway {}.{}.{}.{}, MAC {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}, {patched} patched entries",
            octet(network.interface, 24),
            octet(network.interface, 16),
            octet(network.interface, 8),
            octet(network.interface, 0),
            octet(network.router, 24),
            octet(network.router, 16),
            octet(network.router, 8),
            octet(network.router, 0),
            network.mac[0],
            network.mac[1],
            network.mac[2],
            network.mac[3],
            network.mac[4],
            network.mac[5],
        ));
    }
}

unsafe extern "system" fn hooked_get_adapters_addresses(
    _family: u32,
    _flags: u32,
    reserved: *mut c_void,
    adapters: *mut IP_ADAPTER_ADDRESSES_LH,
    size: *mut u32,
) -> u32 {
    if !reserved.is_null() || size.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let required = size_of::<AdapterBlob>() as u32;
    let available = *size;
    *size = required;
    if adapters.is_null() || available < required {
        return ERROR_BUFFER_OVERFLOW;
    }
    let Some(network) = NETWORK.get() else {
        return ERROR_NOT_SUPPORTED;
    };
    let blob = &mut *(adapters.cast::<AdapterBlob>());
    *blob = zeroed();
    fill_adapter_addresses(blob, network);
    0
}

unsafe extern "system" fn hooked_get_adapters_info(
    adapter: *mut IP_ADAPTER_INFO,
    size: *mut u32,
) -> u32 {
    if size.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let required = size_of::<IP_ADAPTER_INFO>() as u32;
    let available = *size;
    *size = required;
    if adapter.is_null() || available < required {
        return ERROR_BUFFER_OVERFLOW;
    }
    let Some(network) = NETWORK.get() else {
        return ERROR_NOT_SUPPORTED;
    };
    *adapter = zeroed();
    write_ascii(&mut (*adapter).AdapterName, b"Fake Ethernet");
    write_ascii(&mut (*adapter).Description, b"Adapter Description");
    (*adapter).AddressLength = network.mac.len() as u32;
    (&mut (*adapter).Address)[..network.mac.len()].copy_from_slice(&network.mac);
    (*adapter).Index = 1;
    (*adapter).Type = MIB_IF_TYPE_ETHERNET;
    (*adapter).DhcpEnabled = 1;
    fill_ip_addr_string(&mut (*adapter).IpAddressList, network.interface);
    fill_ip_addr_string(&mut (*adapter).GatewayList, network.router);
    fill_ip_addr_string(&mut (*adapter).DhcpServer, network.router);
    (*adapter).LeaseObtained = unix_time() - 3600;
    (*adapter).LeaseExpires = unix_time() + 86400;
    0
}

unsafe extern "system" fn hooked_get_best_route(
    destination: u32,
    source: u32,
    route: *mut MIB_IPFORWARDROW,
) -> u32 {
    if route.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let _ = (destination, source);
    *route = zeroed();
    (*route).dwForwardMask = u32::MAX;
    let Some(network) = NETWORK.get() else {
        return ERROR_NOT_SUPPORTED;
    };
    (*route).dwForwardNextHop = network.router.to_be();
    (*route).dwForwardIfIndex = 1;
    (*route).Anonymous1 = MIB_IPFORWARDROW_0 {
        ForwardType: MIB_IPROUTE_TYPE_INDIRECT,
    };
    (*route).Anonymous2 = MIB_IPFORWARDROW_1 {
        ForwardProto: MIB_IPPROTO_NETMGMT,
    };
    0
}

unsafe extern "system" fn hooked_get_if_table(
    table: *mut MIB_IFTABLE,
    size: *mut u32,
    _order: i32,
) -> u32 {
    if size.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let required = size_of::<u32>() as u32 + size_of::<MIB_IFROW>() as u32;
    let available = *size;
    *size = required;
    if table.is_null() || available < required {
        return ERROR_BUFFER_OVERFLOW;
    }
    let Some(network) = NETWORK.get() else {
        return ERROR_NOT_SUPPORTED;
    };
    ptr::write_bytes(table.cast::<u8>(), 0, required as usize);
    (*table).dwNumEntries = 1;
    let row = (*table).table.as_mut_ptr();
    write_wide(&mut (*row).wszName, "Fake Ethernet");
    (*row).dwIndex = 1;
    (*row).dwType = IF_TYPE_ETHERNET_CSMACD;
    (*row).dwMtu = 4200;
    (*row).dwSpeed = 1_000_000_000;
    (*row).dwPhysAddrLen = network.mac.len() as u32;
    (&mut (*row).bPhysAddr)[..network.mac.len()].copy_from_slice(&network.mac);
    (*row).dwAdminStatus = 1;
    (*row).dwOperStatus = IF_OPER_STATUS_OPERATIONAL;
    0
}

unsafe extern "system" fn hooked_icmp_send_echo_2(
    handle: HANDLE,
    event: HANDLE,
    apc: *mut c_void,
    _context: *mut c_void,
    destination: u32,
    _request: *mut c_void,
    _request_size: u16,
    _options: *mut c_void,
    reply: *mut c_void,
    reply_size: u32,
    _timeout: u32,
) -> u32 {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        applechu::iohook::set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if !apc.is_null() {
        applechu::iohook::set_last_error(ERROR_NOT_SUPPORTED);
        return 0;
    }
    if reply.is_null() {
        applechu::iohook::set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if reply_size < size_of::<ICMP_ECHO_REPLY>() as u32 {
        applechu::iohook::set_last_error(IP_BUF_TOO_SMALL);
        return 0;
    }
    let pong = reply.cast::<ICMP_ECHO_REPLY>();
    *pong = zeroed();
    (*pong).Address = destination;
    (*pong).Status = IP_SUCCESS;
    (*pong).RoundTripTime = 1;
    (*pong).Reserved = 1;
    if !event.is_null() {
        if SetEvent(event) != 0 {
            applechu::iohook::set_last_error(ERROR_IO_PENDING);
        }
        return 0;
    }
    1
}

unsafe extern "system" fn hooked_send_to(
    socket: usize,
    buffer: *const i8,
    length: i32,
    flags: i32,
    destination: *const SOCKADDR,
    destination_len: i32,
) -> i32 {
    let original = original::<SendToFn>(&ORIG_SEND_TO);
    if destination.is_null()
        || (*destination).sa_family != AF_INET
        || destination_len < size_of::<SOCKADDR_IN>() as i32
    {
        return original(socket, buffer, length, flags, destination, destination_len);
    }
    let original_destination = destination.cast::<SOCKADDR_IN>();
    let Some(network) = NETWORK.get() else {
        return original(socket, buffer, length, flags, destination, destination_len);
    };
    let old_broadcast = (network.subnet | 0xFF).to_be();
    if (*original_destination).sin_addr.S_un.S_addr != old_broadcast {
        return original(socket, buffer, length, flags, destination, destination_len);
    }
    let mut replacement = *original_destination;
    replacement.sin_addr.S_un.S_addr = network.broadcast.to_be();
    original(
        socket,
        buffer,
        length,
        flags,
        (&raw const replacement).cast(),
        size_of::<SOCKADDR_IN>() as i32,
    )
}

#[repr(C)]
struct AdapterBlob {
    adapter: IP_ADAPTER_ADDRESSES_LH,
    name: [i8; 64],
    dns_suffix: [u16; 64],
    description: [u16; 64],
    friendly_name: [u16; 64],
    prefix: IP_ADAPTER_PREFIX_XP,
    interface: IP_ADAPTER_UNICAST_ADDRESS_LH,
    router: IP_ADAPTER_GATEWAY_ADDRESS_LH,
    dns: IP_ADAPTER_DNS_SERVER_ADDRESS_XP,
    prefix_address: SOCKADDR_IN,
    interface_address: SOCKADDR_IN,
    router_address: SOCKADDR_IN,
    dns_address: SOCKADDR_IN,
}

unsafe fn fill_adapter_addresses(blob: &mut AdapterBlob, network: &Network) {
    let adapter = &mut blob.adapter;
    adapter.Anonymous1 = IP_ADAPTER_ADDRESSES_LH_0 {
        Anonymous: IP_ADAPTER_ADDRESSES_LH_0_0 {
            Length: size_of::<IP_ADAPTER_ADDRESSES_LH>() as u32,
            IfIndex: 1,
        },
    };
    adapter.AdapterName = blob.name.as_mut_ptr().cast();
    adapter.FirstUnicastAddress = &mut blob.interface;
    adapter.FirstDnsServerAddress = &mut blob.dns;
    adapter.DnsSuffix = blob.dns_suffix.as_mut_ptr();
    adapter.Description = blob.description.as_mut_ptr();
    adapter.FriendlyName = blob.friendly_name.as_mut_ptr();
    adapter.PhysicalAddress[..network.mac.len()].copy_from_slice(&network.mac);
    adapter.PhysicalAddressLength = network.mac.len() as u32;
    adapter.Anonymous2 = IP_ADAPTER_ADDRESSES_LH_1 {
        Flags: IP_ADAPTER_DHCP_ENABLED | IP_ADAPTER_IPV4_ENABLED,
    };
    adapter.Mtu = 4200;
    adapter.IfType = IF_TYPE_ETHERNET_CSMACD;
    adapter.OperStatus = IfOperStatusUp;
    adapter.FirstPrefix = &mut blob.prefix;
    adapter.FirstGatewayAddress = &mut blob.router;
    write_ascii(&mut blob.name, b"{00000000-0000-0000-0000-000000000000}");
    write_wide(&mut blob.dns_suffix, "local");
    write_wide(&mut blob.description, "Interface Description");
    write_wide(&mut blob.friendly_name, "Fake Ethernet");

    blob.interface.Anonymous = IP_ADAPTER_UNICAST_ADDRESS_LH_0 {
        Anonymous: IP_ADAPTER_UNICAST_ADDRESS_LH_0_0 {
            Length: size_of::<IP_ADAPTER_UNICAST_ADDRESS_LH>() as u32,
            Flags: 0,
        },
    };
    blob.interface.Address = socket_address(&mut blob.interface_address);
    blob.interface.PrefixOrigin = IpPrefixOriginDhcp;
    blob.interface.SuffixOrigin = IpSuffixOriginDhcp;
    blob.interface.DadState = IpDadStatePreferred;
    blob.interface.ValidLifetime = u32::MAX;
    blob.interface.PreferredLifetime = u32::MAX;
    blob.interface.LeaseLifetime = 86400;
    blob.interface.OnLinkPrefixLength = 24;

    blob.prefix.Anonymous = IP_ADAPTER_PREFIX_XP_0 {
        Anonymous: IP_ADAPTER_PREFIX_XP_0_0 {
            Length: size_of::<IP_ADAPTER_PREFIX_XP>() as u32,
            Flags: 0,
        },
    };
    blob.prefix.Address = socket_address(&mut blob.prefix_address);
    blob.prefix.PrefixLength = 24;
    blob.router.Anonymous = IP_ADAPTER_GATEWAY_ADDRESS_LH_0 {
        Anonymous: IP_ADAPTER_GATEWAY_ADDRESS_LH_0_0 {
            Length: size_of::<IP_ADAPTER_GATEWAY_ADDRESS_LH>() as u32,
            Reserved: 0,
        },
    };
    blob.router.Address = socket_address(&mut blob.router_address);
    blob.dns.Anonymous = IP_ADAPTER_DNS_SERVER_ADDRESS_XP_0 {
        Anonymous: IP_ADAPTER_DNS_SERVER_ADDRESS_XP_0_0 {
            Length: size_of::<IP_ADAPTER_DNS_SERVER_ADDRESS_XP>() as u32,
            Reserved: 0,
        },
    };
    blob.dns.Address = socket_address(&mut blob.dns_address);
    set_ipv4(&mut blob.prefix_address, network.subnet);
    set_ipv4(&mut blob.interface_address, network.interface);
    set_ipv4(&mut blob.router_address, network.router);
    set_ipv4(&mut blob.dns_address, network.router);
}

fn socket_address(address: &mut SOCKADDR_IN) -> SOCKET_ADDRESS {
    SOCKET_ADDRESS {
        lpSockaddr: (address as *mut SOCKADDR_IN).cast(),
        iSockaddrLength: size_of::<SOCKADDR_IN>() as i32,
    }
}

fn set_ipv4(address: &mut SOCKADDR_IN, value: u32) {
    address.sin_family = AF_INET;
    address.sin_addr.S_un = windows_sys::Win32::Networking::WinSock::IN_ADDR_0 {
        S_addr: value.to_be(),
    };
}

unsafe fn fill_ip_addr_string(target: &mut IP_ADDR_STRING, value: u32) {
    let text = format!(
        "{}.{}.{}.{}",
        octet(value, 24),
        octet(value, 16),
        octet(value, 8),
        octet(value, 0)
    );
    write_ascii(&mut target.IpAddress.String, text.as_bytes());
    write_ascii(&mut target.IpMask.String, b"255.255.255.0");
}

fn write_ascii(buffer: &mut [i8], value: &[u8]) {
    let len = value.len().min(buffer.len().saturating_sub(1));
    for (slot, byte) in buffer.iter_mut().take(len).zip(value) {
        *slot = *byte as i8;
    }
}

fn write_wide(buffer: &mut [u16], value: &str) {
    let len = value
        .encode_utf16()
        .take(buffer.len().saturating_sub(1))
        .count();
    for (slot, unit) in buffer.iter_mut().take(len).zip(value.encode_utf16()) {
        *slot = unit;
    }
}

fn parse_mac(value: &str) -> Option<[u8; 6]> {
    let mut parts = value.split(':').map(|part| u8::from_str_radix(part, 16));
    let address = [
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    ];
    (parts.next().is_none()).then_some(address)
}

fn log_info(message: &str) {
    if let Some(api) = applechu::util::api::API.get() {
        api.log_info(message);
    }
}

fn octet(value: u32, shift: u32) -> u8 {
    (value >> shift) as u8
}

fn unix_time() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn sync_originals() {
    unsafe {
        ORIG_GET_ADAPTERS_ADDRESSES
            .store(ORIG_GET_ADAPTERS_ADDRESSES_PTR as usize, Ordering::Release);
        ORIG_GET_ADAPTERS_INFO.store(ORIG_GET_ADAPTERS_INFO_PTR as usize, Ordering::Release);
        ORIG_GET_BEST_ROUTE.store(ORIG_GET_BEST_ROUTE_PTR as usize, Ordering::Release);
        ORIG_GET_IF_TABLE.store(ORIG_GET_IF_TABLE_PTR as usize, Ordering::Release);
        ORIG_ICMP_SEND_ECHO_2.store(ORIG_ICMP_SEND_ECHO_2_PTR as usize, Ordering::Release);
        ORIG_SEND_TO.store(ORIG_SEND_TO_PTR as usize, Ordering::Release);
    }
}

unsafe fn original<T>(slot: &AtomicUsize) -> T {
    std::mem::transmute_copy(&slot.load(Ordering::Acquire))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_segtools_style_network_values() {
        assert_eq!(parse_ipv4("192.168.139.0"), Some(0xC0A8_8B00));
        assert_eq!(parse_mac("01:02:03:04:05:06"), Some([1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn rejects_incomplete_or_oversized_network_values() {
        assert_eq!(parse_ipv4("192.168.139"), None);
        assert_eq!(parse_ipv4("192.168.139.0.1"), None);
        assert_eq!(parse_mac("01:02:03:04:05"), None);
        assert_eq!(parse_mac("01:02:03:04:05:06:07"), None);
    }
}
