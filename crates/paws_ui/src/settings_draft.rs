//! Profile-bound settings drafts. Runtime publications never overwrite edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DnsDraft {
    pub servers: String,
    pub fallbacks: String,
    pub policy: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VpnDraft {
    pub system_proxy: bool,
    pub dns_hijacking: bool,
    pub allow_bypass: bool,
    pub stack: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NetworkDraft {
    pub mixed_port: String,
    pub controller_port: String,
    pub mixed_enabled: bool,
    pub controller_enabled: bool,
    pub allow_lan: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SettingsValues {
    pub dns: DnsDraft,
    pub vpn: VpnDraft,
    pub network: NetworkDraft,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SettingsBaseline {
    pub profile_id: Option<String>,
    pub revision: u64,
    pub values: SettingsValues,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsSection {
    Dns,
    Vpn,
    Network,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SettingsDraft {
    pub baseline: SettingsBaseline,
    pub values: SettingsValues,
    pub incoming: Option<SettingsBaseline>,
    pub pending: Option<SettingsSection>,
    pub error: Option<String>,
}

impl SettingsDraft {
    pub fn new(baseline: SettingsBaseline) -> Self {
        Self {
            values: baseline.values.clone(),
            baseline,
            incoming: None,
            pending: None,
            error: None,
        }
    }

    pub fn dirty(&self, section: SettingsSection) -> bool {
        match section {
            SettingsSection::Dns => self.values.dns != self.baseline.values.dns,
            SettingsSection::Vpn => self.values.vpn != self.baseline.values.vpn,
            SettingsSection::Network => self.values.network != self.baseline.values.network,
        }
    }

    pub fn observe(&mut self, next: SettingsBaseline) {
        if next.revision < self.baseline.revision
            || self
                .incoming
                .as_ref()
                .is_some_and(|incoming| next.revision < incoming.revision)
        {
            return;
        }
        if next == self.baseline {
            self.incoming = None;
            return;
        }
        if self.pending.is_some() || self.values != self.baseline.values {
            self.incoming = Some(next);
        } else {
            *self = Self::new(next);
        }
    }

    pub fn reload(&mut self) {
        if self.pending.is_some() {
            return;
        }
        if let Some(incoming) = self.incoming.take() {
            *self = Self::new(incoming);
        }
    }

    pub fn begin(&mut self, section: SettingsSection) -> Option<(String, u64, SettingsValues)> {
        if self.pending.is_some() || self.incoming.is_some() || !self.dirty(section) {
            return None;
        }
        let profile_id = self.baseline.profile_id.clone()?;
        self.pending = Some(section);
        self.error = None;
        Some((profile_id, self.baseline.revision, self.values.clone()))
    }

    pub fn finish(&mut self, section: SettingsSection, next: SettingsBaseline) {
        self.pending = None;
        if next.profile_id != self.baseline.profile_id {
            self.incoming = Some(next);
            return;
        }
        // Keep edits in other sections; refresh values the user had not edited.
        if section == SettingsSection::Dns || !self.dirty(SettingsSection::Dns) {
            self.values.dns = next.values.dns.clone();
        }
        if section == SettingsSection::Vpn || !self.dirty(SettingsSection::Vpn) {
            self.values.vpn = next.values.vpn.clone();
        }
        if section == SettingsSection::Network || !self.dirty(SettingsSection::Network) {
            self.values.network = next.values.network.clone();
        }
        let newer = self
            .incoming
            .take()
            .filter(|incoming| incoming.revision > next.revision);
        self.baseline = next;
        self.incoming = newer;
        self.error = None;
    }

    pub fn fail(&mut self, error: String) {
        self.pending = None;
        self.error = Some(error);
    }
}
