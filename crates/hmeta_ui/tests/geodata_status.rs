#[path = "../src/geodata_status.rs"]
mod geodata_status;

use geodata_status::geodata_readiness;
use hmeta_model::GeodataFileSummary;

fn file(name: &str, exists: bool) -> GeodataFileSummary {
    GeodataFileSummary {
        name: name.to_owned(),
        path: format!("/data/geodata/{name}"),
        exists,
        bytes: exists.then_some(1024),
        updated_at: exists.then(|| "1700000000".to_owned()),
    }
}

#[test]
fn geodata_readiness_reports_all_files_available() {
    let files = vec![
        file("GeoIP Country MMDB", true),
        file("GeoLite2 ASN MMDB", true),
        file("GEOSITE MRS", true),
    ];

    let (status, detail) = geodata_readiness(&files);

    assert_eq!(status, "离线资源就绪");
    assert_eq!(detail, "3 个 GeoData 文件均可用");
}

#[test]
fn geodata_readiness_lists_missing_files() {
    let files = vec![
        file("GeoIP Country MMDB", true),
        file("GeoLite2 ASN MMDB", false),
        file("GEOSITE MRS", false),
    ];

    let (status, detail) = geodata_readiness(&files);

    assert_eq!(status, "资源缺失");
    assert_eq!(detail, "缺失：GeoLite2 ASN MMDB, GEOSITE MRS");
}

#[test]
fn geodata_readiness_handles_missing_snapshot_entries() {
    let (status, detail) = geodata_readiness(&[]);

    assert_eq!(status, "未配置");
    assert_eq!(detail, "未发现 GeoData 资源定义");
}
