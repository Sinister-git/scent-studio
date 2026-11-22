use crate::model::Cycle;
use anyhow::{bail, ensure, Result};
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Layout {
    Split,
    Bulk,
    Individual,
}
pub struct Profile {
    pub wifi: bool,
    pub layout: Layout,
    pub cycle: Cycle,
    pub levels: bool,
    pub name: String,
    tail: Vec<u8>,
}
pub fn candidate(n: &str) -> bool {
    n.contains("YGZK-XXJ") || n.contains("BXXXJ-")
}
/// Check the user-supplied PIN locally, as in the app. Never log the advertising buffer.
pub fn verify(n: &str, ad: &[u8], entered: &[u8]) -> Result<Profile> {
    ensure!(
        candidate(n) && ad.len() > 22,
        "Unrecognized or incomplete diffuser advertisement"
    );
    let wifi = n.contains("-BLEWIFI-");
    let individual = n.contains("-XXJV") || n.contains("-XXJ-2");
    let dc = n.contains("-DC-") || n.contains("-DCL-");
    let layout = if wifi || dc || n.contains("-V3NF-") || n.contains("-V3TJ-") {
        Layout::Split
    } else if individual {
        Layout::Individual
    } else if ["-TJ-", "-DCTJ-", "-TJUN-"].iter().any(|p| n.contains(p)) {
        Layout::Bulk
    } else {
        bail!("Unrecognized firmware variant: {n}")
    };
    let matches = if individual && !wifi {
        entered == b"9999"
    } else {
        let (start, end) = if wifi { (23, 27) } else { (22, 28) };
        ensure!(
            ad.len() >= if wifi { 27 } else { 26 },
            "Incomplete device advertisement"
        );
        let field: Vec<_> = ad[start..end.min(ad.len())]
            .iter()
            .copied()
            .filter(|&b| b != 0)
            .collect();
        if field.is_empty() {
            entered == b"9999"
        } else {
            field == entered
        }
    };
    ensure!(matches, "Incorrect PIN for selected device");
    let mut tail = vec![];
    if dc {
        tail.extend([0, 0])
    }
    if individual {
        tail.extend_from_slice(&ad[4..6])
    }
    let ty = if wifi && ad.len() == 28 { ad[27] } else { 0 };
    let music = n.contains("-BLEWIFI-M");
    let cycle = if layout == Layout::Bulk || (wifi && ty == 1 && !music) {
        Cycle::Short
    } else if dc {
        Cycle::Dc(if n.contains("-DCL-") { 25 } else { 10 })
    } else {
        Cycle::Standard
    };
    Ok(Profile {
        wifi,
        layout,
        cycle,
        levels: individual || dc || n.contains("-V3TJ-") || (wifi && ty == 0 && !music),
        name: n.into(),
        tail,
    })
}
fn sum(v: &[u8]) -> u8 {
    v.iter().fold(0, |a, b| a.wrapping_add(*b))
}
pub fn frame(op: u8, data: Option<&[u8]>) -> Vec<u8> {
    let mut f = vec![85, 16, op];
    if let Some(d) = data {
        f.extend([5, d.len() as u8]);
        f.extend(d)
    } else {
        f.push(0)
    }
    f.push(sum(&f[1..]));
    f
}
impl Profile {
    pub fn request(&self, op: u8, data: Option<&[u8]>) -> Vec<u8> {
        let mut v = frame(op, data);
        v.extend(&self.tail);
        v
    }
    pub fn response<'a>(&self, op: u8, f: &'a [u8]) -> Result<&'a [u8]> {
        let data = response(op, f).or_else(|_| {
            let f = f
                .strip_suffix(self.tail.as_slice())
                .ok_or_else(|| anyhow::anyhow!("Invalid reply trailer"))?;
            response(op, f)
        })?;
        Ok(data)
    }
}
pub fn response(op: u8, f: &[u8]) -> Result<&[u8]> {
    ensure!(
        f.len() >= 6 && f[..3] == [85, 17, op] && f.len() == 6 + f[4] as usize,
        "Invalid reply header or size"
    );
    let end = f.len() - 1;
    ensure!(sum(&f[1..end]) == f[end], "Invalid reply checksum");
    Ok(&f[5..end])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pin() {
        let mut ad = [0; 28];
        ad[23..27].copy_from_slice(b"1234");
        assert!(verify("BXXXJ-BLEWIFI-X", &ad, b"9999").is_err());
        assert!(verify("BXXXJ-BLEWIFI-X", &ad, b"1234").is_ok());
        assert!(verify("BXXXJ-BLEWIFI-X", &ad[..25], b"1234").is_err());
    }
    #[test]
    fn replies() {
        let p = verify("BXXXJ-DC-X", &[0; 28], b"9999").unwrap();
        assert!(p.request(6, None).ends_with(&[0, 0]));
        assert_eq!(
            p.response(17, &[85, 17, 17, 5, 1, 1, 41, 0, 0]).unwrap(),
            [1]
        );
        assert!(response(17, &[85, 17, 17, 5, 1, 1, 40]).is_err());
        assert_eq!(frame(0xf1, None), [85, 16, 241, 0, 1]);
    }
}
