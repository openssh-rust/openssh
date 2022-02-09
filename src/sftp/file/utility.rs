use std::io::IoSlice;
use std::ops::Deref;

use bytes::Bytes;
use tokio_io_utility::IoSliceExt;

/// Return `Some((n, subslices, reminder))` where
///  - `n` is number of bytes in `subslices` and `reminder`.
///  - `subslices` is a subslice of `bufs`
///  - `reminder` might be a slice of `bufs[subslices.len()]`
///    if `subslices.len() < bufs.len()` and the total number
///    of bytes in `subslices` is less than `limit`.
///
/// Return `None` if the total number of bytes in `bufs` is empty.
fn take_slices<T: Deref<Target = [u8]>>(
    bufs: &'_ [T],
    limit: usize,
    create_slice: impl FnOnce(&T, usize) -> T,
) -> Option<(usize, &'_ [T], [T; 1])> {
    if bufs.is_empty() {
        return None;
    }

    let mut end = 0;
    let mut n = 0;

    // loop 'buf
    //
    // This loop would skip empty `IoSlice`s.
    for buf in bufs {
        let cnt = n + buf.len();

        // branch '1
        if cnt > limit {
            break;
        }

        n = cnt;
        end += 1;
    }

    let buf = if end < bufs.len() {
        // In this branch, the loop 'buf terminate due to branch '1,
        // thus
        //
        //     n + buf.len() > limit,
        //     buf.len() > limit - n.
        //
        // And (limit - n) also cannot be 0, otherwise
        // branch '1 will not be executed.
        let res = [create_slice(&bufs[end], limit - n)];

        n = limit;

        res
    } else {
        if n == 0 {
            return None;
        }

        [create_slice(&bufs[0], 0)]
    };

    Some((n, &bufs[..end], buf))
}

/// Return `Some((n, io_subslices, [reminder]))` where
///  - `n` is number of bytes in `io_subslices` and `reminder`.
///  - `io_subslices` is a subslice of `io_slices`
///  - `reminder` might be a slice of `io_slices[io_subslices.len()]`
///    if `io_subslices.len() < io_slices.len()` and the total number
///    of bytes in `io_subslices` is less than `limit`.
///
/// Return `None` if the total number of bytes in `io_slices` is empty.
pub(super) fn take_io_slices<'a>(
    io_slices: &'a [IoSlice<'a>],
    limit: usize,
) -> Option<(usize, &'a [IoSlice<'a>], [IoSlice<'a>; 1])> {
    take_slices(io_slices, limit, |io_slice, end| {
        IoSlice::new(&io_slice.into_inner()[..end])
    })
}

/// Return `Some((n, bytes_subslice, [reminder]))` where
///  - `n` is number of bytes in `bytes_subslice` and `reminder`.
///  - `bytes_subslice` is a subslice of `bytes_slice`
///  - `reminder` might be a slice of `bytes_slice[bytes_subslice.len()]`
///    if `bytes_subslice.len() < bytes_slice.len()` and the total number
///    of bytes in `bytes_subslice` is less than `limit`.
///
/// Return `None` if the total number of bytes in `bytes_slice` is empty.
pub(super) fn take_bytes(
    bytes_slice: &[Bytes],
    limit: usize,
) -> Option<(usize, &[Bytes], [Bytes; 1])> {
    take_slices(bytes_slice, limit, |bytes, end| bytes.slice(0..end))
}

#[cfg(test)]
mod tests {
    use super::{take_io_slices, IoSlice};
    use pretty_assertions::assert_eq;

    #[test]
    fn test_take_io_slices() {
        let limit = 200;

        let content = b"HELLO, WORLD!\n".repeat(limit / 8);
        let len = content.len();

        assert!(len / 2 < limit);

        let io_slices = [
            IoSlice::new(&content[..len / 2]),
            IoSlice::new(&content[len / 2..]),
        ];

        let (n, io_subslices, reminder) = take_io_slices(&io_slices, limit).unwrap();

        assert_eq!(n, limit);
        assert_eq!(io_subslices.len(), 1);
        assert_eq!(&*io_subslices[0], &*io_slices[0]);
        assert_eq!(&*reminder[0], &io_slices[1][..(limit - len / 2)]);
    }
}
