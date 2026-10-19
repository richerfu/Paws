use super::*;

fn backup_client(url: &str) -> Result<(reqwest::Client, reqwest::Url), PawsError> {
    let parsed =
        reqwest::Url::parse(url).map_err(|_| PawsError::Core("Invalid WebDAV URL".into()))?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(PawsError::Core(
            "WebDAV requires HTTPS without embedded credentials/query".into(),
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| PawsError::Core("Cannot create WebDAV client".into()))?;
    Ok((client, parsed))
}

impl CoreHandle {
    /// Explicit user action only. Refuse overwriting an existing remote backup.
    pub async fn upload_webdav_backup(
        &self,
        url: &str,
        user: &str,
        password: &str,
    ) -> Result<(), PawsError> {
        let (client, url) = backup_client(url)?;
        let backup = self.export_portable_backup()?;
        if backup.len() > 20 * 1024 * 1024 {
            return Err(PawsError::Core("Backup exceeds 20 MiB".into()));
        }
        let mut request = client
            .put(url)
            .header("Content-Type", "application/json")
            .header("If-None-Match", "*")
            .body(backup);
        if !user.is_empty() {
            request = request.basic_auth(user, Some(password));
        }
        let response = request
            .send()
            .await
            .map_err(|_| PawsError::Core("WebDAV upload failed or timed out".into()))?;
        if !response.status().is_success() {
            return Err(PawsError::Core(format!(
                "WebDAV upload HTTP {} (existing file will not be overwritten)",
                response.status().as_u16()
            )));
        }
        Ok(())
    }
    pub async fn restore_webdav_backup(
        &self,
        url: &str,
        user: &str,
        password: &str,
    ) -> Result<usize, PawsError> {
        let (client, url) = backup_client(url)?;
        let mut request = client.get(url);
        if !user.is_empty() {
            request = request.basic_auth(user, Some(password));
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| PawsError::Core("WebDAV download failed or timed out".into()))?;
        if !response.status().is_success() {
            return Err(PawsError::Core(format!(
                "WebDAV download HTTP {}",
                response.status().as_u16()
            )));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| PawsError::Core("WebDAV download interrupted".into()))?
        {
            if body.len() + chunk.len() > 20 * 1024 * 1024 {
                return Err(PawsError::Core("Backup exceeds 20 MiB".into()));
            }
            body.extend_from_slice(&chunk);
        }
        let text =
            String::from_utf8(body).map_err(|_| PawsError::Core("Backup is not UTF-8".into()))?;
        self.restore_portable_backup(&text).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn webdav_rejects_insecure_or_embedded_credentials() {
        for url in [
            "http://example.com/backup",
            "https://user:pass@example.com/backup",
            "https://example.com/backup?token=secret",
            "file:///tmp/backup",
        ] {
            assert!(backup_client(url).is_err());
        }
        assert!(backup_client("https://example.com/TinyPaws.json").is_ok());
    }
}
