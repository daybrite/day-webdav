// SPDX-License-Identifier: MPL-2.0
//! Applications supply localized labels, enrollment policy, and secure persistence.
use day::prelude::*;

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
            label(name),
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

/// Password is transient and cleared on submit. The caller must validate the collection
/// with the proposed credentials before persisting them.
pub fn account_form(
    account: AccountFields,
    configured: impl Fn() -> bool + Copy + 'static,
    busy: impl Fn() -> bool + Copy + 'static,
    labels: Labels,
    submit: impl Fn(String, String, String, String) + Copy + 'static,
) -> impl Piece {
    let (base, initial_folder) = account.address();
    let url = Signal::new(base);
    let folder = Signal::new(initial_folder);

    let username = Signal::new(account.username);
    let invalid = Signal::new(false);
    let password = Signal::new(String::new());
    let valid = move || {
        !busy()
            && !password.get().is_empty()
            && !username.get().trim().is_empty()
            && !url.get().trim().is_empty()
    };
    let save = move || {
        if busy() {
            return;
        }
        let name = folder.get_untracked().trim().to_owned();
        let Ok(endpoint) = target(url.get_untracked().trim(), &name) else {
            invalid.set(true);
            return;
        };
        invalid.set(false);
        let secret = password.get_untracked();
        password.set(String::new());
        submit(
            endpoint,
            name,
            username.get_untracked().trim().into(),
            secret,
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
