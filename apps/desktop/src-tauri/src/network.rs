use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct NetworkPolicy {
    pub status: &'static str,
    pub unmetered_wifi: bool,
    pub reason: String,
}
impl NetworkPolicy {
    fn unknown(reason: impl Into<String>) -> Self {
        Self {
            status: "unknown",
            unmetered_wifi: false,
            reason: reason.into(),
        }
    }
}
#[cfg(windows)]
pub fn policy() -> NetworkPolicy {
    use windows::{
        Networking::Connectivity::{NetworkCostType, NetworkInformation},
        Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
    };
    let initialized = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let owns_apartment = initialized.is_ok();
    if let Err(error) = initialized {
        if error.code().0 != 0x80010106u32 as i32 {
            return NetworkPolicy::unknown("WinRT 网络状态不可用");
        }
    }
    struct Apartment(bool);
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { RoUninitialize() };
            }
        }
    }
    let _apartment = Apartment(owns_apartment);
    let result = (|| -> windows::core::Result<NetworkPolicy> {
        let profile = NetworkInformation::GetInternetConnectionProfile()?;
        let cost = profile.GetConnectionCost()?;
        let wifi = profile.IsWlanConnectionProfile()?;
        let unmetered = cost.NetworkCostType()? == NetworkCostType::Unrestricted
            && !cost.Roaming()?
            && !cost.OverDataLimit()?
            && !cost.ApproachingDataLimit()?
            && !cost.BackgroundDataUsageRestricted()?;
        Ok(NetworkPolicy {
            status: if wifi && unmetered {
                "unmetered_wifi"
            } else {
                "restricted"
            },
            unmetered_wifi: wifi && unmetered,
            reason: if !wifi {
                "当前互联网连接并非 Wi-Fi"
            } else if !unmetered {
                "Wi-Fi 被系统标记为计费或受限"
            } else {
                "Windows 确认为非计费 Wi-Fi"
            }
            .into(),
        })
    })();
    result.unwrap_or_else(|_| NetworkPolicy::unknown("无法可靠识别 Wi-Fi，自动升级已暂停"))
}
#[cfg(not(windows))]
pub fn policy() -> NetworkPolicy {
    NetworkPolicy::unknown("此平台尚未实现可靠的非计费 Wi-Fi 检测")
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_network_query_keeps_unknown_and_restricted_connections_ineligible() {
        let policy = super::policy();
        assert_eq!(policy.unmetered_wifi, policy.status == "unmetered_wifi");
        assert!(!policy.reason.is_empty());
        println!("{}", serde_json::to_string(&policy).unwrap());
    }
}
