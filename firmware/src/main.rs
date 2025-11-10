use anyhow::{Context, Result};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition, timer::EspTimerService,
    wifi::EspWifi,
};
use std::default::Default;

const WIFI_SSID: &str = include_str!("../config_ssid.txt");
const WIFI_PASSWORD: &str = include_str!("../config_password.txt");

fn main() -> Result<()> {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    // Now you can use the log macros!
    log::set_max_level(log::LevelFilter::Debug);

    log::info!("Setting up the eventfd...");
    let eventfd_config = esp_idf_sys::esp_vfs_eventfd_config_t {
        max_fds: 1,
        ..Default::default()
    };
    esp_idf_sys::esp_nofail! { unsafe { esp_idf_sys::esp_vfs_eventfd_register(&eventfd_config) } }

    log::info!("Setting up the board...");
    let peripherals = esp_idf_hal::peripherals::Peripherals::take()?;
    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let timer_service = EspTimerService::new()?;

    let esp_wifi = EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs))
        .expect("failed to get esp_wifi");
    let mut wifi = esp_idf_svc::wifi::AsyncWifi::wrap(esp_wifi, sys_loop, timer_service)
        .expect("failed to wrap wifi");

    log::info!("Starting async run loop");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            start_wifi(&mut wifi).await.expect("couldn't start wifi");
            update_time().await.expect("couldn't update time");
            display_url().await.expect("couldn't download file");
        });

    log::info!("complete");

    Ok(())
}

async fn display_url() -> Result<()> {
    let body = reqwest::get("http://example.com").await?.text().await?;

    log::info!("{}", body);

    Ok(())
}

fn format_time() -> Result<String, time::error::Format> {
    time::UtcDateTime::now().format(time::macros::format_description!(
        "[day]-[month repr:short]-[year] [hour]:[minute]:[second]"
    ))
}

async fn update_time() -> Result<()> {
    let client = esp_idf_svc::sntp::EspSntp::new(&esp_idf_svc::sntp::SntpConf {
        servers: ["pool.ntp.org"],
        operating_mode: esp_idf_svc::sntp::OperatingMode::Poll,
        sync_mode: esp_idf_svc::sntp::SyncMode::Immediate,
    })?;

    while client.get_sync_status() != esp_idf_svc::sntp::SyncStatus::Completed {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    log::info!("ntp syncing completed, current time: {}", format_time()?);
    Ok(())
}

async fn start_wifi(wifi: &mut esp_idf_svc::wifi::AsyncWifi<EspWifi<'static>>) -> Result<()> {
    // Connect to WiFi
    let ssid: heapless::String<32> = heapless::String::try_from(WIFI_SSID).expect("invalid ssid");
    let password: heapless::String<64> =
        heapless::String::try_from(WIFI_PASSWORD).expect("invalid password");

    wifi.set_configuration(&esp_idf_svc::wifi::Configuration::Client(
        esp_idf_svc::wifi::ClientConfiguration {
            ssid: ssid.parse().unwrap(),
            auth_method: esp_idf_svc::wifi::AuthMethod::WPA2Personal,
            password: password.parse().unwrap(),
            ..Default::default()
        },
    ))?;

    wifi.start().await.context("wifi couldn't start")?;
    wifi.connect().await.context("wifi couldn't connect")?;
    wifi.wait_netif_up().await.context("wifi netif_up failed")?;

    let net_if = wifi.wifi().sta_netif();
    log::info!(
        "nameservers {}, {}",
        net_if.get_dns(),
        net_if.get_secondary_dns()
    );

    Ok(())
}
