# UI action migration and behavior ledger

Baseline: Paws `8d4579c` (`Action` had 94 variants). This ledger is an
exhaustive inventory: every former intent and completion action is mapped to
its replacement owner and to behavioral evidence. A completion row marked
"callback" is deliberately not another event: the operation returns a typed
`Result`, and its owner applies the receipt only while the relevant page/root
generation is alive.

Evidence abbreviations:

- `RD`: `cargo test -p paws_ui --test reactive_domains --locked` (real Dioxus
  `VirtualDom`; a focused signal-isolation harness, equal-write suppression,
  production Tokio-provider cancellation before/after first poll, and bounded
  slow-consumer backpressure). It validates the subscription mechanism, not a
  full rendered end-to-end test of every application page.
- `SD`: `cargo test -p paws_ui --test settings_draft --locked` (profile and
  revision-bound drafts, conflict and pending behavior).
- `UI`: the named `paws_ui` integration test plus the full UI integration run.
- `CORE`: the named `paws_core`/`paws_profile` behavioral test.
- `PLATFORM`: `vpn_startup_contract` plus `node --test
  scripts/test-vpn-platform.mjs`; actual device behavior remains a separate
  release check.

## Runtime, preferences and VPN (1-11)

| # | Former action | Replacement owner | Behavioral evidence |
|---:|---|---|---|
| 1 | `RefreshSnapshot` | Root `subscribe_runtime_revisions`; fetch only the announced typed projection. | RD; CORE `snapshot_reads_are_pure_and_keep_the_same_revision`. |
| 2 | `SnapshotLoaded` | `UiStores::apply_{config,status,resource,telemetry}_projection`, each version checked and structurally published. | RD; CORE `explicit_telemetry_refresh_publishes_a_new_telemetry_revision`. |
| 3 | `TickSnapshot` | No UI timer/full snapshot. The core sampler publishes a telemetry revision; the root provider pulls telemetry only. | RD; `vpn_extension_subscription_is_event_driven_without_polling`. |
| 4 | `LogRecordingStatusLoaded` | Initial log projection plus `set_log_recording` receipt; the recording error is retained in `LogsProjection` and rendered on the logs page. | UI `log_recording_is_opt_in_with_daily_history_and_export`. |
| 5 | `SetLanguagePreference` | `UiServices::set_language` updates only `PreferencesProjection`, persists, and notifies. | UI `preferences_round_trip_and_keep_system_defaults`, `platform_changes_notify_without_telemetry_and_survive_root_replacement`. |
| 6 | `SetThemePreference` | `UiServices::set_theme`; color-mode ACK has pending/error/retry fields in the preferences domain. | UI `appearance_settings_use_arkit_shadcn_choices_and_persist_actions`, system-preference test. |
| 7 | `StartStopVpn` | `UiServices::toggle_vpn` reads the session/profile projections but owns progress in the narrow `UiOperationStores.vpn` signal. A unique UI operation id gates duplicate toggles; status projections cannot settle bridge work. | PLATFORM `vpn_start_does_not_reload_the_already_active_profile_or_poll_state`; UI VPN operation-state tests. |
| 8 | `VpnCommandFinished` | Start, stop, owned-stop and restart carry process-unique request ids and recoverable bridge receipts. Only the matching terminal receipt clears its operation; an unconfirmed result stops the indefinite spinner but keeps the gate and offers same-receipt confirmation or an explicit stop-and-resynchronize fence. A failed recovery stop restores the prior unknown blocker, and stale callbacks cannot clear a newer operation. | PLATFORM operation-receipt/lookup tests; UI false-status, duplicate, stale completion, unconfirmed and recovery tests. |
| 9 | `VpnStateEvent` | One root-owned revisioned runtime provider; platform frames are owned and monotonic in core. | PLATFORM `platform_vpn_state_uses_one_event_pump_and_in_process_subscribers`; CORE `attempt_scoped_callbacks_reject_stale_and_post_terminal_updates`. |
| 10 | `SetMode` | `UiServices::set_mode`; a monotonic request generation suppresses an older prepare completion. | CORE `mode_changes_are_reflected`, `global_mode_is_rejected_without_an_active_tunnel`; UI dashboard mode test. |
| 11 | `ModeChanged` | Direct completion refreshes config/status and publishes localized feedback only for the current generation. | Same mode tests; generation guard is exercised by the architecture acceptance review. |

## Proxy, diagnostics and activity (12-23)

| # | Former action | Replacement owner | Behavioral evidence |
|---:|---|---|---|
| 12 | `SelectProxy` | `UiServices::select_proxy(group, Some(proxy))`, with a proxy-domain pending marker. | CORE `stale_proxy_selection_completion_is_rejected_before_persistence`; UI `proxy_selection_updates_only_the_exact_rule_group`. |
| 13 | `UnfixProxy` | The same operation with `None`; selection identity remains group-scoped. | CORE `automatic_group_pins_and_auto_mode_persist_across_reload`; `proxy_grid` tests. |
| 14 | `ProxySelected` | Direct `Result`; resource/status revisions provide the authoritative selected member. | CORE `selected_proxy_and_global_node_are_restored_after_reload`; RD. |
| 15 | `TestAllProxyDelays` | `UiServices::test_all_proxy_delays`; pending belongs to the narrow `UiOperationStores.proxy` signal rather than the proxy data projection. | UI `proxy_delay_test_has_visible_pending_feedback`; core resource operation sequencing tests. |
| 16 | `AllProxyDelaysTested` | Direct batch result, then status/resource projection refresh and aggregate feedback. | UI proxy-delay contract; CORE `newer_resource_operation_supersedes_an_older_completion`. |
| 17 | `FlushDnsCache` | Concrete `run_diagnostic("dns", ...)`; only diagnostics/telemetry are refreshed. | CORE `dns_snapshot_exposes_tun_cache_diagnostics`; diagnostics UI suite. |
| 18 | `FlushFakeIpCache` | Concrete `run_diagnostic("fakeip", ...)`. | Diagnostics UI suite and controller behavioral coverage. |
| 19 | `HealthcheckProxyProvider` | Concrete provider-key diagnostic operation. | CORE `provider_refresh_disambiguates_same_name_by_type`; resources UI contract. |
| 20 | `HealthcheckProviderProxy` | Concrete provider/member-key operation with URL/status inputs. | CORE controller/provider registry tests; resources UI contract. |
| 21 | `ControllerDiagnosticFinished` | Direct `Result`; the narrow diagnostics-operation signal clears and telemetry refreshes. | RD signal-isolation mechanism regression plus source-level domain ownership review; diagnostics UI suite. |
| 22 | `CloseConnection` | `UiServices::close_connection`; durable core mutation followed by telemetry refresh. | Activity UI actions; CORE connection/telemetry tests. |
| 23 | `ConnectionClosed` | Direct typed callback and notification; no whole-state replacement. | UI `virtual_activity_rows_keep_their_previous_actions`; CORE telemetry revision behavior. |

## Rule lookup and manual rule editor (24-39)

| # | Former action | Replacement owner | Behavioral evidence |
|---:|---|---|---|
| 24 | `OpenRuleLookup` | Page-local `LocalRuleEditors` Signal allocates a new lookup id. | UI `resources_header_opens_a_domain_and_ip_rule_lookup`; RD isolation. |
| 25 | `CloseRuleLookup` | Cancels the owned Tokio query and removes only the local draft. | UI `lookup_state_tracks_async_results_without_reopening_a_closed_dialog`; RD unmount cancellation. |
| 26 | `SetRuleLookupQuery` | Direct local Signal update; blocked while submitting. | Same lookup test; RD equal-write behavior. |
| 27 | `LookupRule` | Page query with abort handle, alive guard and lookup id guard. | UI lookup lifecycle test; CORE `rule_lookup_uses_compiled_rule_order_independently_of_runtime_mode`. |
| 28 | `AddRuleFromLookup` | Converts the typed result into a local manual-rule draft. | UI rule lookup + manual-rule tests. |
| 29 | `RuleLookedUp` | Direct query completion accepted only for the same local lookup id. | UI lookup lifecycle test. |
| 30 | `OpenManualRuleEditor` | Page-local `ManualRuleEditorState`; no application store field. | UI `activity_rows_create_structured_hot_rules_without_leaving_virtual_lists`; RD. |
| 31 | `CloseManualRuleEditor` | Removes the page-local draft unless its mutation is pending. | Manual-rule UI contract; page-owned state. |
| 32 | `SetManualRuleMatchKind` | Local draft update plus normalized selector/value. | `manual_rule::previews_normalize_domains_and_host_prefixes`. |
| 33 | `SetManualRuleValue` | Local draft update. | Manual-rule normalization tests; RD. |
| 34 | `SetManualRuleTarget` | Local draft update. | Manual-rule conflict/target tests. |
| 35 | `SetManualRuleDisconnect` | Local draft update. | Activity-row manual-rule contract. |
| 36 | `SaveManualRule` | Concrete durable `apply_manual_rule`; optional connection close is a separately reported result. | CORE `manual_activity_rules_persist_and_hot_update_the_existing_tunnel`. |
| 37 | `ManualRuleSaved` | Direct result; config/telemetry refresh, local dialog close on success, error retained on failure. | CORE manual activity rule test; manual-rule UI tests. |
| 38 | `CloseAllConnections` | `UiServices::close_all_connections`; concrete durable operation. | UI activity virtual-row action test; controller behavior suite. |
| 39 | `AllConnectionsClosed` | Direct callback, telemetry refresh and localized feedback. | Activity UI suite; typed telemetry publication (RD). |

## External links, request history and logs (40-49)

| # | Former action | Replacement owner | Behavioral evidence |
|---:|---|---|---|
| 40 | `OpenExternalUrl` | `UiServices::open_external_url`; root-owned bridge future. | UI converter/about/privacy contracts. |
| 41 | `ExternalUrlOpened` | Direct bridge result; only failures publish a notification. | UI `converter_page_exposes_sub_web_actions_and_privacy_context`. |
| 42 | `ClearRequestHistory` | Concrete core mutation in `UiServices::clear_request_history`. | Activity filter/virtual-list suite; CORE immediate telemetry-revision coverage. |
| 43 | `RequestHistoryCleared` | Direct result, telemetry refresh, success/error notification. | Same activity/core evidence. |
| 44 | `ToggleLogRecording` | `UiOperationStores.logs.recording_pending` + concrete persistence task; the large logs projection is not invalidated for spinner state. | UI `log_recording_is_opt_in_with_daily_history_and_export`. |
| 45 | `LogRecordingChanged` | Direct receipt updates only `LogsProjection`; telemetry refreshes archive data and carries the recording-source error, so only a successful recording retry clears it. | Same log contract. |
| 46 | `ExportLogArchive` | Concrete picker/export mutation with archive-name pending identity. | Log contract and diagnostics virtual list. |
| 47 | `LogArchiveExported` | Direct callback clears exact pending state and reports result. | Log contract. |
| 48 | `DeleteLogArchive` | Concrete core deletion with archive-name pending identity. | Log contract. |
| 49 | `LogArchiveDeleted` | Typed receipt includes the new recording status; only logs/telemetry update. | CORE `clear_logs_removes_state_and_runtime_logs`; log contract. |

## Profile import and profile lifecycle (50-72)

| # | Former action | Replacement owner | Behavioral evidence |
|---:|---|---|---|
| 50 | `ResetProfileImportFeedback` | `UiServices::reset_profile_import_feedback`, narrow profile-import operation signal only. | UI network-import lifecycle test. |
| 51 | `CancelProfileImport` | Generation bump plus watch cancellation applies only to picker/download/validation preparation. Once commit begins it remains runtime-owned and only the callback is suppressed. | UI `profile_import_can_close_while_pending_and_discards_stale_results`, timeout/cancellation contract; CORE import rollback tests. |
| 52 | `ImportLocalProfile` | Typed picker `Option` distinguishes cancellation from real errors; validation returns an opaque prepared import before durable commit. | UI profile-export/import bridge contracts; profile import tests. |
| 53 | `ScanProfileSubscription` | Scan cancellation is an explicit empty result; new/existing subscriptions use cancellable prepare followed by revision-checked durable import/refresh-and-activate. | UI subscription-scan tests; PLATFORM verifies cancellation versus an error containing “cancel”; CORE stale config mutation test. |
| 54 | `LocalProfileImportFinished` | Direct generation-checked completion applies the commit receipt and restarts only the captured session owner at the receipt revision. | UI import lifecycle/cancellation tests; PLATFORM owned restart test. |
| 55 | `ImportProfileFromUrl` | Timeout/cancellation wraps only download/normalize/validate; `commit_prepared_profile_import_and_activate_checked` is never inside `select!`. | UI network-import lifecycle and timeout contracts; CORE profile import rollback tests. |
| 56 | `ProfileImportFinished` | The preparation callback starts one durable commit only if its generation is current; stale/closed UI cannot reopen or overwrite, while an already-started commit is not dropped. | UI stale import completion contract; CORE transaction rollback tests. |
| 57 | `ImportRules` | Page-owned query performs picker + pure parse; the UI rechecks active profile/revision before a separate durable checked commit/reload and receipt-scoped restart. | Profile `imports_custom_rules_from_text_and_clash_yaml`; CORE checked rule import rollback; resources UI contract. |
| 58 | `RulesImported` | Direct receipt callback updates config/resource/status; route disposal suppresses the callback without aborting a started commit, and pending clears separately. | CORE checked rule import rollback; PLATFORM owned restart contract. |
| 59 | `ActivateProfile` | `UiServices::activate_profile`; `activate_profile_checked` consumes the captured config revision and returns the projection used for owner-scoped restart. | CORE `failed_activation_keeps_the_previous_profile_active`. |
| 60 | `ProfileActivated` | Direct typed result; config/status refresh and restart-aware feedback. | CORE profile activation/reload tests; PLATFORM owned restart. |
| 61 | `DeleteProfile` | Core transactional deletion. Replacement uses owner+revision restart; deleting the final profile uses owner+revision checked stop. | CORE `deleting_active_profile_reloads_next_or_clears_engine`, profile rollback tests; PLATFORM checked stop. |
| 62 | `ProfileDeleted` | Direct `ProfileDeleteResult` keeps storage success distinct from VPN restart/stop failure. | CORE deletion tests; checked-stop platform harness. |
| 63 | `RefreshProfile` | Concrete durable subscription refresh. | Profile `refresh_success_and_failure_metadata_persist_with_profile`. |
| 64 | `ProfileRefreshed` | Direct callback refreshes config and preserves per-profile error feedback. | Same profile test and profile filter tests. |
| 65 | `RefreshAllProfiles` | Concrete batch refresh; attempted identities captured before launch. | CORE `refresh_all_profiles_continues_after_single_failure`. |
| 66 | `ProfilesRefreshed` | Direct result calculates failure count from the returned/published profile domain. | Same core test; profile refresh feedback tests. |
| 67 | `RestoreProfileBackup` | Single core transaction already reloads the active runtime; UI requests only an owner-scoped platform restart. | CORE `profile_edit_and_backup_restore_reload_active_tunnel`; profile checkpoint test. |
| 68 | `ProfileBackupRestored` | Direct result; no second activation/reload, restart failure is reported separately. | Same core/profile tests. |
| 69 | `UpdateProfileSubscription` | `update_profile_subscription_checked(profile, expected_revision, ...)`. | CORE `checked_profile_mutation_rejects_a_stale_revision_without_writing`; profile subscription edit test. |
| 70 | `ProfileSubscriptionUpdated` | Config projection transaction receipt applied directly; no full snapshot. | Same checked mutation tests; RD. |
| 71 | `ExportProfile` | YAML read plus concrete document-picker export. | UI `profile_export_reaches_the_harmony_document_picker`. |
| 72 | `ProfileExported` | Direct callback, success/error notification only. | Same export contract. |

## YAML editor, providers and rules (73-88)

| # | Former action | Replacement owner | Behavioral evidence |
|---:|---|---|---|
| 73 | `OpenYamlEditor` | `load_yaml_editor_draft` runs once on open and binds profile id + config revision. | UI YAML summary tests; RD editor isolation. |
| 74 | `SetYamlEditorOpen` | `Signal<Option<YamlEditorDraft>>`; closing drops the page/dialog owner. | RD provider/page cancellation. |
| 75 | `SetYamlEditorText` | Local dialog Signal only. | RD focused signal-isolation regression plus source-level verification that the YAML draft is route-local. |
| 76 | `ResetYamlEditorText` | Local draft resets to its captured original. | YAML summary changed/unchanged tests. |
| 77 | `TestYamlEditor` | Page-owned cancellable query; no durable mutation. | YAML parser/profile validation suite; RD cancellation. |
| 78 | `YamlEditorTested` | Direct completion guarded by page lifetime and profile identity. | RD unmount cancellation and YAML summary tests. |
| 79 | `SaveYamlEditor` | Revision-checked durable content transaction plus owner-scoped restart. | CORE stale config mutation and profile edit/reload tests. |
| 80 | `YamlEditorSaved` | Direct typed completion; apply config projection, close only matching draft, retain failure locally. | Same core tests; RD isolation. |
| 81 | `RefreshProvider` | `refresh_provider_checked(type, name, expected_resource_revision)`. | CORE `checked_provider_refresh_rejects_a_stale_resource_revision`. |
| 82 | `ProviderRefreshed` | Resource projection receipt is applied directly; no snapshot fanout. | CORE resource operation sequencing; RD. |
| 83 | `RefreshAllProviders` | Concrete core batch operation; resource/status only. | CORE empty/inline-only provider batch tests. |
| 84 | `ProvidersRefreshed` | Direct batch result with attempted provider identities and persisted per-provider errors. | Provider batch core tests and resources UI contract. |
| 85 | `SetRuleEnabled` | `set_rule_enabled_checked(profile, expected_revision, rule, enabled)`. | CORE checked config mutation; resources UI contract. |
| 86 | `ReorderRules` | `reorder_rules_checked`; one transaction owns persistence and active-runtime reload. | Profile `rule_reorder_updates_runtime_yaml_order`; CORE stale mutation test. |
| 87 | `DeleteRule` | `delete_rule_checked`; one transaction owns persistence and active-runtime reload. | CORE/profile rule transaction suite. |
| 88 | `RulesChanged` | Direct config receipt and owner-scoped restart; no duplicate activation/reload. | Checked mutation tests; PLATFORM owned restart. |

## Settings (89-94)

| # | Former action | Replacement owner | Behavioral evidence |
|---:|---|---|---|
| 89 | `SaveDnsSettings` | Page-local `SettingsForm` calls `set_profile_dns_config_checked` with profile id and base revision. | SD; CORE `dns_config_updates_reload_active_snapshot`. |
| 90 | `DnsSettingsSaved` | Transaction receipt advances only that draft section if profile/revision still match. | SD late receipt and concurrent section tests. |
| 91 | `SaveVpnSettings` | Page-local form calls checked VPN mutation. Unsupported `allow_bypass` and `system_proxy` are not presented as working switches; legacy true values require explicit clearing. | SD; CORE `vpn_config_updates_reload_active_snapshot`; settings UI capability contract. |
| 92 | `VpnSettingsSaved` | Direct receipt plus owner+revision platform restart; external changes become explicit conflicts. | SD; PLATFORM owned restart. |
| 93 | `SaveNetworkSettings` | Page-local form calls checked network mutation with validated ports/access. | SD; profile `network_ports_and_controller_access_are_profile_scoped_and_validated`. |
| 94 | `NetworkSettingsSaved` | Direct receipt, section-scoped pending completion and owner-scoped restart. | SD; CORE external-controller reload convergence test. |

## Architecture acceptance

- There is no application-wide `State`, `Action`, `Command`, reducer, dispatch
  channel, manual invalidation mask, full-snapshot polling loop, dialog content
  hash, or fixed 40 ms modal switch delay in production UI code.
- `App` and `AppShell` do not read a combined runtime model. Pages subscribe to
  the smallest practical typed projections; YAML, settings, search/filter and
  dialog drafts are local Signals. Transient VPN/import/proxy/diagnostic/log
  busy state lives in narrow operation Signals rather than the core data domains.
  VPN bridge ownership is likewise separate from the session lifecycle:
  `vpn_running = false` is not a stop receipt and cannot unlock another start.
- Query lifetime is page-owned and cancellable. Durable mutations continue
  through their core commit/rollback boundary, while stale root/page callbacks
  are rejected by Dioxus/runtime generation and local identity guards.
- Projection application rejects older domain revisions and uses structural
  equality before writing. Resource/config/status/telemetry revisions do not
  impersonate one another. Tokio subscriptions cross into Dioxus through a
  bounded channel; scope disposal aborts them even before their local observer
  is first polled. A successful unrelated projection can clear only the exact
  prior projection error, not a bootstrap or preferences error.

Device validation also exposed that ArkUI virtual-list rows are detached
`VirtualDom`s and cannot inherit contexts from their page. Resource/editor,
log-history and proxy rows now receive Signals, callbacks and resolved colors
explicitly; detached rows do not mount context-dependent spinners.

QEMU validation additionally exposed a stop-cleanup race: the Extension could
publish `vpn_running = false` while the submitted Stop was still waiting for
its cleanup acknowledgement. VPN pending state is now receipt-owned rather
than projection-owned, so that intermediate lifecycle update cannot enqueue a
Start behind the unresolved Stop.

Final host UI verification on 2026-09-12 (all 94 rows remain accounted for):

- `cargo check -p paws_ui --lib --locked`: passed.
- `cargo check -p paws_ui --tests --locked`: passed (all UI test targets
  type-checked).
- VPN operation model: **7 passed, 0 failed**, included in the normal Cargo
  integration target using the production module directly. This avoids linking
  the HarmonyOS-only native `ohfileshare` dependency into a macOS lib-test binary.
- All `paws_ui` integration tests: **138 passed, 0 failed**.
- Final single-command workspace coverage is **409 Rust tests** plus **34 platform
  behavioral tests**. The exact command history and separate device acceptance
  are recorded in [dioxus-state-audit-remediation.md](dioxus-state-audit-remediation.md).
