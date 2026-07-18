use hmeta_model::GeodataFileSummary;

pub(crate) fn geodata_readiness(files: &[GeodataFileSummary]) -> (&'static str, String) {
    if files.is_empty() {
        return ("未配置", "未发现 GeoData 资源定义".to_owned());
    }

    let missing = files
        .iter()
        .filter(|file| !file.exists)
        .map(|file| file.name.as_str())
        .collect::<Vec<_>>();

    if missing.is_empty() {
        (
            "离线资源就绪",
            format!("{} 个 GeoData 文件均可用", files.len()),
        )
    } else {
        ("资源缺失", format!("缺失：{}", missing.join(", ")))
    }
}
