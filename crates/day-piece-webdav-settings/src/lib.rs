// SPDX-License-Identifier: MPL-2.0
//! Applications supply localized labels, enrollment policy, and secure persistence.
use day::prelude::*;
use day_piece_settings::{Section, rows};

/// Localized headings and field labels for the native settings sections.
pub struct SectionLabels {
    /// Heading above the server URL and application folder.
    pub server_section: day::LocalizedText,
    /// Heading above authentication and verification controls.
    pub account_section: day::LocalizedText,
    /// Instructions displayed below the server fields.
    pub help: day::LocalizedText,
    /// Labels shared with the standalone account form.
    pub fields: Labels,
}

pub struct Labels {
    pub endpoint: day::LocalizedText,
    pub folder: day::LocalizedText,
    pub invalid_address: day::LocalizedText,
    pub username: day::LocalizedText,
    pub password: day::LocalizedText,
    pub connect: day::LocalizedText,
    pub update: day::LocalizedText,
}

fn account_field(name: day::LocalizedText, control: impl Piece) -> impl Piece {
    if day::size_class().is_some_and(|c| c.width == WidthClass::Compact) {
        let accessible_name = name.clone();
        column((
            label(name).font(Font::Footnote).secondary(),
            control
                .a11y(move |a| a.label(accessible_name))
                .grow(Dimension::Width),
        ))
        .align(HAlign::Leading)
        .spacing(Space::XS)
        .grow(Dimension::Width)
        .any()
    } else {
        labeled(name, control).any()
    }
}

/// A saved account retains the user's choice of where the service URL ends.
pub struct AccountFields {
    pub endpoint: String,
    pub saved_folder: Option<String>,
    pub default_folder: &'static str,
    pub username: String,
}

const SERVERS: &[&str] = &[
    "https://myfiles.fastmail.com/",
    "https://app.koofr.net/dav/Koofr/",
    "https://webdav.pcloud.com/",
    "https://ewebdav.pcloud.com/",
];

fn folder_path(folder: &str) -> Result<day_webdav::ResourcePath, day_webdav::Error> {
    if folder.is_empty() {
        return Ok(day_webdav::ResourcePath::root());
    }
    day_webdav::ResourcePath::from_segments(folder.split('/'), true)
}

fn target(server: &str, folder: &str) -> Result<String, day_webdav::Error> {
    Ok(day_webdav::Endpoint::loopback(server)?.url(&folder_path(folder)?))
}

impl AccountFields {
    fn address(&self) -> (String, String) {
        if self.endpoint.is_empty() {
            return (String::new(), self.default_folder.into());
        }
        let folder = self.saved_folder.as_deref().unwrap_or(self.default_folder);
        if !folder.is_empty()
            && let Ok(path) = folder_path(folder)
            && let Some(base) = self.endpoint.strip_suffix(path.as_encoded_str())
            && base.ends_with('/')
            && target(base, folder).ok().as_ref() == Some(&self.endpoint)
        {
            return (base.into(), folder.into());
        }
        (self.endpoint.clone(), String::new())
    }
}

fn server_input(
    url: Signal<String>,
    locked: impl Fn() -> bool + Copy + 'static,
    label: day::LocalizedText,
) -> impl Piece {
    if cfg!(target_os = "ios") {
        let choices = SERVERS
            .iter()
            .map(|&s| menu_item(s).action(move || url.set(s.into())))
            .collect::<Vec<_>>();
        row((
            text_field(url)
                .input_purpose(InputPurpose::Url)
                .read_only(locked)
                .id(day::id!(dav_endpoint))
                .grow(Dimension::Width),
            menu_button(label, choices)
                .icon(Symbol::Down)
                .icon_only()
                .enabled(move || !locked())
                .id(day::id!(dav_servers)),
        ))
        .grow(Dimension::Width)
        .any()
    } else {
        let servers = Signal::new(SERVERS.iter().map(|s| (*s).to_owned()).collect());
        day::combobox::combo_box(servers, url)
            .enabled(move || !locked())
            .id(day::id!(dav_endpoint))
            .grow(Dimension::Width)
            .any()
    }
}

#[derive(Clone, Copy)]
struct Draft {
    url: Signal<String>,
    folder: Signal<String>,
    username: Signal<String>,
    password: Signal<String>,
    invalid: Signal<bool>,
}
impl Draft {
    fn new(account: AccountFields) -> Self {
        let (base, folder) = account.address();
        Self {
            url: Signal::new(base),
            folder: Signal::new(folder),
            username: Signal::new(account.username),
            password: Signal::new(String::new()),
            invalid: Signal::new(false),
        }
    }
    fn ready(self, busy: bool, can_reuse_password: bool) -> bool {
        !busy
            && (!self.password.get().is_empty() || can_reuse_password)
            && !self.username.get().trim().is_empty()
            && !self.url.get().trim().is_empty()
    }
    fn save(
        self,
        busy: bool,
        saved_password: impl FnOnce(&str, &str) -> Option<String>,
        submit: impl FnOnce(String, String, String, String),
    ) {
        if busy {
            return;
        }
        let folder = self.folder.get_untracked().trim().to_owned();
        let Ok(endpoint) = target(self.url.get_untracked().trim(), &folder) else {
            self.invalid.set(true);
            return;
        };
        self.invalid.set(false);
        let username = self.username.get_untracked().trim().to_owned();
        let mut password = self.password.get_untracked();
        if password.is_empty() {
            let Some(saved) = saved_password(&endpoint, &username).filter(|s| !s.is_empty()) else {
                return;
            };
            password = saved;
        }
        self.password.set_if_changed(password.clone());
        submit(endpoint, folder, username, password);
    }
    fn refresh(self, new: &((String, String), String), old: Option<&((String, String), String)>) {
        if Some(new) == old {
            return;
        }
        let ((base, folder), username) = new;
        self.url.set_if_changed(base.clone());
        self.folder.set_if_changed(folder.clone());
        self.username.set_if_changed(username.clone());
        self.password.set_if_changed(String::new());
        self.invalid.set_if_changed(false);
    }
}
/// Native settings sections with independent rows, rather than a form nested inside a row.
/// A blank password reuses the saved credential only for the same endpoint and username.
/// Saved credentials remain visible as masked dots. The caller verifies edits before saving.
pub fn account_sections(
    account: impl Fn() -> AccountFields + Copy + 'static,
    saved_password: impl Fn() -> String + Copy + 'static,
    configured: impl Fn() -> bool + Copy + 'static,
    busy: impl Fn() -> bool + Copy + 'static,
    labels: SectionLabels,
    submit: impl Fn(String, String, String, String) + Copy + 'static,
) -> Vec<Section> {
    let SectionLabels {
        server_section,
        account_section,
        help,
        fields: labels,
    } = labels;
    let draft = Draft::new(account());
    let password_mounted = Signal::new(false);
    let Draft {
        url,
        folder,
        username,
        password,
        invalid,
    } = draft;
    watch(
        move || {
            let saved = account();
            (saved.address(), saved.username)
        },
        move |new, old| {
            draft.refresh(new, old);
            if Some(new) != old && password_mounted.get_untracked() {
                draft.password.set_if_changed(saved_password());
            }
        },
    );
    let same_account = move |endpoint: &str, username: &str| {
        let saved = account();
        configured() && saved.endpoint == endpoint && saved.username == username
    };
    let valid = move || {
        draft.ready(
            busy(),
            configured() && account().username == username.get().trim(),
        )
    };
    let save = move || {
        draft.save(
            busy(),
            |endpoint, username| same_account(endpoint, username).then(saved_password),
            submit,
        )
    };
    let draft_owner = day::reactive::Scope::current();
    let endpoint_label = labels.endpoint.clone();
    let server = Section::new()
        .title(server_section)
        .footer(help)
        .row(rows::custom(move || {
            account_field(
                endpoint_label.clone(),
                server_input(url, move || configured() || busy(), endpoint_label.clone()),
            )
        }))
        .row(rows::custom(move || {
            account_field(
                labels.folder.clone(),
                text_field(folder)
                    .input_purpose(InputPurpose::Url)
                    .read_only(move || configured() || busy())
                    .id(day::id!(dav_folder))
                    .grow(Dimension::Width),
            )
        }));
    let credentials = Section::new()
        .title(account_section)
        .row(rows::custom(move || {
            account_field(
                labels.username.clone(),
                text_field(username)
                    .input_purpose(InputPurpose::Username)
                    .read_only(busy)
                    .id(day::id!(dav_username))
                    .grow(Dimension::Width),
            )
        }))
        .row(rows::custom(move || {
            if password.get_untracked().is_empty() {
                password.set_if_changed(saved_password());
            }
            password_mounted.set(true);
            day::reactive::Scope::current().on_cleanup(move || {
                if draft_owner.is_alive() {
                    password_mounted.set(false);
                    password.set_if_changed(String::new());
                }
            });
            account_field(
                labels.password.clone(),
                secure_field(password)
                    .read_only(busy)
                    .submit_label(SubmitLabel::Go)
                    .on_submit(move || {
                        if valid() {
                            save();
                        }
                    })
                    .id(day::id!(dav_password))
                    .grow(Dimension::Width),
            )
        }))
        .row(
            rows::action(
                move || {
                    if configured() {
                        labels.update.format()
                    } else {
                        labels.connect.format()
                    }
                },
                save,
            )
            .enabled(valid)
            .id(day::id!(dav_connect)),
        )
        .footer(move || {
            if invalid.get() {
                labels.invalid_address.format()
            } else {
                String::new()
            }
        });
    vec![server, credentials]
}

/// Password is transient and cleared on submit. The caller must validate the collection
/// with the proposed credentials before persisting them.
pub fn account_form(
    account: AccountFields,
    configured: impl Fn() -> bool + Copy + 'static,
    busy: impl Fn() -> bool + Copy + 'static,
    labels: Labels,
    submit: impl Fn(String, String, String, String) + Copy + 'static,
) -> impl Piece {
    let draft = Draft::new(account);
    let Draft {
        url,
        folder,
        username,
        password,
        invalid,
    } = draft;
    let valid = move || draft.ready(busy(), false);
    let save = move || {
        draft.save(
            busy(),
            |_, _| None,
            move |url, folder, user, secret| {
                password.set(String::new());
                submit(url, folder, user, secret);
            },
        );
    };
    column((
        form((section((
            account_field(
                labels.endpoint.clone(),
                server_input(url, move || configured() || busy(), labels.endpoint.clone()),
            ),
            account_field(
                labels.folder,
                text_field(folder)
                    .read_only(move || configured() || busy())
                    .id(day::id!(dav_folder))
                    .grow(Dimension::Width),
            ),
            account_field(
                labels.username,
                text_field(username)
                    .read_only(busy)
                    .id(day::id!(dav_username))
                    .grow(Dimension::Width),
            ),
            account_field(
                labels.password,
                secure_field(password)
                    .id(day::id!(dav_password))
                    .grow(Dimension::Width),
            ),
        )),))
        .grow(Dimension::Width),
        when(
            move || invalid.get(),
            move || label(labels.invalid_address.clone()).id(day::id!(dav_invalid_address)),
        ),
        when(configured, move || {
            button(labels.update.clone())
                .prominent()
                .enabled(valid)
                .action(save)
                .id(day::id!(dav_update_credentials))
                .grow(Dimension::Width)
        }),
        when(
            move || !configured(),
            move || {
                button(labels.connect.clone())
                    .prominent()
                    .enabled(valid)
                    .action(save)
                    .id(day::id!(dav_connect))
                    .grow(Dimension::Width)
            },
        ),
    ))
    .spacing(Space::M)
    .grow(Dimension::Width)
}

/// Show a disclosure only while the host has a failure to explain. The host formats its
/// redacted diagnostic on the UI thread. A new failure after a retry starts collapsed.
pub fn failure_details(
    title: day::LocalizedText,
    detail: impl Fn() -> Option<String> + Copy + 'static,
) -> impl Piece {
    when(
        move || detail().is_some(),
        move || {
            let expanded = Signal::new(false);
            column((
                button(title.clone())
                    .action(move || expanded.set(!expanded.get_untracked()))
                    .id(day::id!(dav_more_info)),
                when(
                    move || expanded.get(),
                    move || {
                        label(move || detail().unwrap_or_default()).id(day::id!(dav_error_detail))
                    },
                ),
            ))
            .align(HAlign::Leading)
            .spacing(Space::XS)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_account() -> AccountFields {
        AccountFields {
            endpoint: "https://fixture.invalid/books/".into(),
            saved_folder: Some("books".into()),
            default_folder: "books",
            username: "synthetic-user".into(),
        }
    }
    #[test]
    fn drafting_and_unchanged_credentials_have_separate_lifetimes() {
        let draft = Draft::new(fixture_account());
        let saved = (fixture_account().address(), fixture_account().username);
        draft.password.set("synthetic edit".into());
        draft.folder.set("edited folder".into());
        draft.refresh(&saved, Some(&saved));
        assert_eq!(draft.folder.get_untracked(), "edited folder");
        draft.save(
            false,
            |_, _| panic!("a typed password must not read secure storage"),
            |_, _, _, secret| assert_eq!(secret, "synthetic edit"),
        );
        assert_eq!(draft.password.get_untracked(), "synthetic edit");
        draft.refresh(&saved, None);
        draft.save(
            false,
            |endpoint, user| {
                assert_eq!(endpoint, fixture_account().endpoint);
                assert_eq!(user, fixture_account().username);
                Some("synthetic saved".into())
            },
            |_, _, _, secret| assert_eq!(secret, "synthetic saved"),
        );
        assert_eq!(draft.password.get_untracked(), "synthetic saved");
        draft.password.set(String::new());
        draft.save(
            false,
            |_, _| Some(String::new()),
            |_, _, _, _| panic!("a keychain miss must not submit a blank credential"),
        );
        draft.password.set("superseded".into());
        let mut changed = saved.clone();
        changed.1 = "other-user".into();
        draft.refresh(&changed, Some(&saved));
        assert_eq!(draft.username.get_untracked(), "other-user");
        assert!(draft.password.get_untracked().is_empty());
    }
    #[test]
    fn building_native_sections_does_not_read_the_keychain() {
        // Generated labels are supplied by host applications. These keys are synthetic fixtures.
        let label = || day::tr("fixture-label");
        let labels = SectionLabels {
            server_section: label(),
            account_section: label(),
            help: label(),
            fields: Labels {
                endpoint: label(),
                folder: label(),
                invalid_address: label(),
                username: label(),
                password: label(),
                connect: label(),
                update: label(),
            },
        };
        let sections = account_sections(
            fixture_account,
            || panic!("no keychain read until the password row mounts"),
            || true,
            || false,
            labels,
            |_, _, _, _| {},
        );
        assert_eq!(sections.len(), 2);
    }
    #[test]
    fn defaults_only_apply_to_new_accounts_and_existing_urls_round_trip() {
        let mut fields = AccountFields {
            endpoint: String::new(),
            saved_folder: None,
            default_folder: "Stanza-Redux",
            username: String::new(),
        };
        assert_eq!(fields.address(), (String::new(), "Stanza-Redux".into()));
        for (old, server, folder) in [
            (
                "https://fixture.invalid/files/Stanza-Redux/",
                "https://fixture.invalid/files/",
                "Stanza-Redux",
            ),
            (
                "https://fixture.invalid/files/",
                "https://fixture.invalid/files/",
                "",
            ),
        ] {
            fields.endpoint = old.into();
            assert_eq!(fields.address(), (server.into(), folder.into()));
            assert_eq!(target(server, folder).unwrap(), old);
        }
    }

    #[test]
    fn addresses_round_trip_without_reinterpreting_user_text() {
        for folder in ["", "Stanza-Redux", "My books/日本 #1", "100%"] {
            let base = "https://fixture.invalid/remote.php/dav/files/user/";
            let full = target(base, folder).unwrap();
            let fields = AccountFields {
                endpoint: full.clone(),
                saved_folder: Some(folder.into()),
                default_folder: "Stanza-Redux",
                username: String::new(),
            };
            let (server, name) = fields.address();
            assert_eq!(server, base);
            assert_eq!(name, folder);
            assert_eq!(target(&server, &name).unwrap(), full);
        }
        for folder in [".", "../books", "/books", "a//b", "a\\b"] {
            assert!(target("https://fixture.invalid/", folder).is_err());
        }
        assert!(target("https://user:secret@fixture.invalid/", "books").is_err());
    }
}
