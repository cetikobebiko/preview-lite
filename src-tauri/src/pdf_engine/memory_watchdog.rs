//! PDFium worker'ları için bellek bekçisi.
//!
//! Linux'ta worker kendi adres alanını `RLIMIT_AS` ile sınırlar. macOS'ta bu sınır
//! uygulanamaz (süreç başlarken zaten yüzlerce GiB sanal alan eşler) ve `ps` RSS'i
//! sıkıştırıcı/swap yüzünden gerçek kullanımı göstermez. Bu yüzden ebeveyn süreç,
//! çalışan worker'ın `phys_footprint` değerini periyodik okur ve bütçeyi aşan worker'ı
//! öldürür. Windows'ta Job Object sınırlaması yoktur; orada bekçi ölçüm yapmaz.

use std::time::Duration;

/// macOS'taki ayak izi bütçesi: Linux'taki `RLIMIT_AS` tavanının aynısı (en fazla üç
/// render tamponu, kimlik bilgili belge kopyası ve PDFium payı).
#[cfg(target_os = "macos")]
pub(crate) const WORKER_FOOTPRINT_LIMIT_BYTES: u64 =
    crate::worker::pdfium::WORKER_ADDRESS_SPACE_LIMIT_BYTES;
#[cfg(not(target_os = "macos"))]
pub(crate) const WORKER_FOOTPRINT_LIMIT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Ayak izi saniyede ~1.3 GB büyüyebilir; 100 ms'lik aralıkta aşım ~130 MB'ta kalır.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Sürecin fiziksel bellek ayak izi (bayt). Süreç okunamazsa ya da platform
/// desteklemiyorsa `None`.
#[cfg(target_os = "macos")]
pub(crate) fn phys_footprint(pid: u32) -> Option<u64> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: proc_pid_rusage, RUSAGE_INFO_V2 için tam bir rusage_info_v2 yazar; tampon
    // o boyutta ve hizada. Hata durumunda okumayız.
    let status = unsafe {
        libc::proc_pid_rusage(
            pid,
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    // SAFETY: status 0 ise yapı çekirdek tarafından dolduruldu.
    (status == 0).then(|| unsafe { info.assume_init() }.ri_phys_footprint)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn phys_footprint(_pid: u32) -> Option<u64> {
    None
}

/// Ayak izi `limit`'i aşıyorsa ölçülen değeri döndürür.
pub(crate) fn over_limit(pid: u32, limit: u64) -> Option<u64> {
    phys_footprint(pid).filter(|footprint| *footprint > limit)
}

/// Worker'ın ayak izi `limit`'i aşana kadar bekler ve ölçülen değeri döndürür.
/// Ölçüm yapılamayan platformlarda ya da pid yoksa hiç tamamlanmaz; çağıran bunu
/// yanıt future'ıyla yarıştırır.
pub(crate) async fn wait_until_over_limit(pid: Option<u32>, limit: u64) -> u64 {
    let Some(pid) = pid.filter(|_| cfg!(target_os = "macos")) else {
        return std::future::pending().await;
    };
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        if let Some(footprint) = over_limit(pid, limit) {
            return footprint;
        }
    }
}

pub(crate) fn mib(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_matches_the_linux_address_space_ceiling() {
        assert_eq!(WORKER_FOOTPRINT_LIMIT_BYTES, 2 * 1024 * 1024 * 1024);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_the_footprint_of_a_child_that_touches_memory() {
        // A separate process, so other tests' allocations cannot move the number.
        let mut child = std::process::Command::new("perl")
            .arg("-e")
            .arg("my $x = 'a' x (384 * 1024 * 1024); sleep 30;")
            .spawn()
            .unwrap();
        let pid = child.id();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut footprint = 0;
        while std::time::Instant::now() < deadline {
            footprint = phys_footprint(pid).unwrap_or(0);
            if footprint > 256 * 1024 * 1024 {
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        let _ = child.kill();
        let _ = child.wait();

        assert!(footprint > 256 * 1024 * 1024, "footprint {footprint}");
        assert!(over_limit(pid, u64::MAX).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn missing_process_has_no_footprint() {
        assert_eq!(phys_footprint(u32::MAX), None);
    }

    #[tokio::test]
    async fn without_a_pid_the_watchdog_never_fires() {
        let fired = tokio::time::timeout(POLL_INTERVAL * 3, wait_until_over_limit(None, 0)).await;
        assert!(fired.is_err());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn fires_once_the_process_is_over_the_limit() {
        let footprint = tokio::time::timeout(
            Duration::from_secs(2),
            wait_until_over_limit(Some(std::process::id()), 1),
        )
        .await
        .expect("own process is above one byte");
        assert!(footprint > 1);
    }
}
