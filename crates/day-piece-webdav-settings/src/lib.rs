// SPDX-License-Identifier: MPL-2.0
//! Applications supply localized labels, enrollment policy, and secure persistence.
use day::prelude::*;

pub struct Labels {
    pub endpoint: day::LocalizedText,
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

/// Password is transient and cleared on submit. The caller must validate the collection
/// with the proposed credentials before persisting them.
pub fn account_form(
    endpoint: String,
    user: String,
    configured: impl Fn() -> bool + Copy + 'static,
    busy: impl Fn() -> bool + Copy + 'static,
    labels: Labels,
    submit: impl Fn(String, String, String) + Copy + 'static,
) -> impl Piece {
    let url = Signal::new(endpoint);
    let username = Signal::new(user);
    let password = Signal::new(String::new());
    let valid = move || {
        !busy()
            && !password.get().is_empty()
            && !username.get().trim().is_empty()
            && !url.get().trim().is_empty()
    };
    let save = move || {
        let secret = password.get_untracked();
        password.set(String::new());
        submit(
            url.get_untracked().trim().into(),
            username.get_untracked().trim().into(),
            secret,
        );
    };
    column((
        form((section((
            account_field(
                labels.endpoint,
                text_field(url)
                    .input_purpose(InputPurpose::Url)
                    .read_only(move || configured() || busy())
                    .id(day::id!(dav_endpoint))
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
