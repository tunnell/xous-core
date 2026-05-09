use smoltcp::wire::IpAddress;

use crate::*;

pub(crate) fn parse_address(data: &[u8]) -> Option<smoltcp::wire::IpAddress> {
    let mut i = data.iter();
    match i.next() {
        Some(&4) => Some(smoltcp::wire::IpAddress::v4(*i.next()?, *i.next()?, *i.next()?, *i.next()?)),
        Some(&6) => {
            let mut new_addr = [0u8; 16];
            for octet in new_addr.iter_mut() {
                *octet = *i.next()?;
            }
            let v6: std::net::Ipv6Addr = new_addr.into();
            Some(v6.into())
        }
        _ => None,
    }
}

pub(crate) fn write_address(address: IpAddress, data: &mut [u8]) -> Option<usize> {
    let mut i = data.iter_mut();
    match address {
        IpAddress::Ipv4(a) => {
            *i.next()? = 4;
            for (dest, src) in i.zip(a.as_bytes().iter()) {
                *dest = *src;
            }
            Some(5)
        }
        IpAddress::Ipv6(a) => {
            *i.next()? = 6;
            for (dest, src) in i.zip(a.as_bytes().iter()) {
                *dest = *src;
            }
            Some(16)
        }
    }
}

pub(crate) fn respond_with_error(mut env: xous::MessageEnvelope, code: NetError) -> Option<()> {
    // If it's not a memory message, don't fill in the return information.
    let body = match env.body.memory_message_mut() {
        None => {
            // But do respond to the scalar message, if it's a BlockingScalar
            if env.body.scalar_message().is_some() && env.body.is_blocking() {
                xous::return_scalar(env.sender, code as usize).ok();
            }
            return None;
        }
        Some(b) => b,
    };

    body.valid = None;
    let s: &mut [u8] = unsafe { body.buf.as_slice_mut() };
    let mut i = s.iter_mut();
    // NetError is not Copy; bind to u8 once.
    let code_u8 = code as u8;

    // Duplicate error to ensure it's seen as an error regardless of byte order/return type
    // This is necessary because errors are encoded as `u8` slices, but "good"
    // responses may be encoded as `u16` or `u32` slices.
    //
    // Image-18 fix (image-17 diagnosis): the std-side Xous net backend
    // disagrees with itself about which byte the error code lives in.
    //   library/std/src/sys/net/connection/xous/tcpstream.rs SEND path
    //     reads `send_request.raw[4]` for the code (matches the
    //     historical [1,1,1,1, code, 0,0,0] layout below).
    //   library/std/src/sys/net/connection/xous/tcpstream.rs RECV path,
    //     udp.rs RECV path, and tcplistener.rs ACCEPT path all read
    //     `result[1]` for the code — which used to always be 1, so
    //     ErrorKind::TimedOut and ErrorKind::WouldBlock were
    //     unreachable from the recv side.
    //
    // The receive-side bug surfaced in xas as a death-spiral on
    // every WebSocket: the 5s read_timeout we set fires
    // respond_with_error(NetError::TimedOut) on the kernel side, but
    // std mapped it to "recv_slice failure" generic IO error instead
    // of ErrorKind::TimedOut. ws_pump treated that as fatal and tore
    // down the WS, libsignal saw the WsClosing, manager spawned a
    // fresh WS, repeat every 5s.
    //
    // Fix: also write the code at byte 1 (where the buggy std-recv
    // looks) in addition to byte 4 (where std-send looks). Both
    // call sites now decode correctly. The "buf as u32 LE != 0"
    // marker still holds because byte 0 is still 1.
    *i.next()? = 1;             // byte 0 — error marker
    *i.next()? = code_u8;    // byte 1 — code (where std-recv reads)
    *i.next()? = 1;             // byte 2 — marker
    *i.next()? = 1;             // byte 3 — marker
    *i.next()? = code_u8;    // byte 4 — code (where std-send reads)
    *i.next()? = 0;
    *i.next()? = 0;
    *i.next()? = 0;
    None
}

pub(crate) fn respond_with_connected(
    mut env: xous::MessageEnvelope,
    idx: u16,
    local_port: u16,
    remote_port: u16,
) {
    let body = env.body.memory_message_mut().unwrap();
    let bfr = unsafe { body.buf.as_slice_mut::<u16>() };

    log::debug!("successfully connected: {}", idx);
    bfr[0] = 0;
    bfr[1] = idx;
    bfr[2] = local_port;
    bfr[3] = remote_port;
}

/// Insert `Some(value)` into the first slot in the Vec that is `None`,
/// or append it to the end if there is no free slot
pub(crate) fn insert_or_append<T>(arr: &mut Vec<Option<T>>, val: T) -> usize {
    // Look for a free index, or add it onto the end.
    let mut idx = None;
    for (i, elem) in arr.iter_mut().enumerate() {
        if elem.is_none() {
            idx = Some(i);
            break;
        }
    }
    if let Some(idx) = idx {
        arr[idx] = Some(val);
        idx
    } else {
        let idx = arr.len();
        arr.push(Some(val));
        idx
    }
}
