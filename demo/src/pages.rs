// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
use crate::{
    model::{Entry, Files, Operation, Preview, Scene, size_text},
    res,
};
use day::prelude::*;
fn status(s: Scene) -> impl Piece {
    column((
        label(s.status).id(day::id!(operation_status)),
        when(
            move || s.busy.get(),
            move || {
                button(res::str::cancel())
                    .action(move || s.cancel())
                    .id(day::id!(cancel_operation))
            },
        ),
    ))
    .spacing(Space::S)
    .align(HAlign::Leading)
}
fn provider(s: Scene) -> AnyPiece {
    #[cfg(all(not(target_arch = "wasm32"), not(target_env = "ohos")))]
    {
        labeled(
            res::str::provider(),
            picker(
                s.provider,
                [
                    (0, res::str::provider_native()),
                    (1, res::str::provider_reqwest()),
                ],
            )
            .segmented()
            .id(day::id!(http_provider)),
        )
        .any()
    }
    #[cfg(any(target_arch = "wasm32", target_env = "ohos"))]
    {
        let _ = s;
        label(res::str::provider_native_only())
            .id(day::id!(native_only))
            .any()
    }
}
pub fn connection() -> impl Piece {
    let s = Scene::ambient();
    scroll(
        column((
            label(res::str::connection_title())
                .font(Font::Title2)
                .bold()
                .id(day::id!(connection_title)),
            label(res::str::connection_help()).secondary(),
            section((
                provider(s),
                labeled(
                    res::str::endpoint(),
                    text_field(s.endpoint)
                        .min_width(180.0)
                        .id(day::id!(dav_endpoint)),
                ),
                labeled(
                    res::str::auth_kind(),
                    picker(
                        s.auth,
                        [
                            (0, res::str::auth_none()),
                            (1, res::str::auth_basic()),
                            (2, res::str::auth_bearer()),
                        ],
                    )
                    .id(day::id!(auth_kind)),
                ),
                labeled(
                    res::str::username(),
                    text_field(s.username)
                        .min_width(180.0)
                        .id(day::id!(dav_username)),
                ),
                labeled(
                    res::str::password(),
                    secure_field(s.password)
                        .min_width(180.0)
                        .id(day::id!(dav_password)),
                ),
                button(res::str::fixture())
                    .action(move || s.fixture())
                    .id(day::id!(use_fixture)),
                label(res::str::fixture_help())
                    .font(Font::Footnote)
                    .secondary(),
            )),
            section((
                labeled(
                    res::str::allow_writes(),
                    toggle(s.writes).id(day::id!(allow_writes)),
                ),
                labeled(
                    res::str::constrained(),
                    toggle(s.constrained).id(day::id!(constrained_network)),
                ),
                labeled(
                    res::str::expensive(),
                    toggle(s.expensive).id(day::id!(allow_expensive)),
                ),
                label(res::str::policy_help())
                    .font(Font::Footnote)
                    .secondary(),
            )),
            row((
                button(res::str::options())
                    .enabled(move || !s.busy.get())
                    .action(move || s.run(Operation::Options))
                    .id(day::id!(server_options)),
                button(res::str::smoke())
                    .enabled(move || !s.busy.get())
                    .action(move || s.run(Operation::Smoke))
                    .id(day::id!(run_checks)),
            ))
            .spacing(Space::S)
            .fit(RowFit::Wrap { run_spacing: 8.0 }),
            label(res::str::smoke_help())
                .font(Font::Footnote)
                .secondary(),
            status(s),
        ))
        .spacing(Space::L)
        .padding(Space::L)
        .align(HAlign::Leading)
        .grow(Dimension::Width),
    )
}
pub fn browser() -> impl Piece {
    let s = Scene::ambient();
    scroll(
        column((
            label(res::str::nav_browser())
                .font(Font::Title2)
                .bold()
                .id(day::id!(browser_title)),
            label(res::str::browser_help()).secondary(),
            labeled(
                res::str::path(),
                text_field(s.path)
                    .min_width(180.0)
                    .id(day::id!(resource_path)),
            ),
            row((
                button(res::str::browse())
                    .action(move || s.run(Operation::List))
                    .id(day::id!(list_resources)),
                button(res::str::get())
                    .action(move || s.run(Operation::Get))
                    .id(day::id!(download_resource)),
                button(res::str::head())
                    .action(move || s.run(Operation::Head))
                    .id(day::id!(head_resource)),
            ))
            .spacing(Space::S)
            .fit(RowFit::Wrap { run_spacing: 8.0 }),
            when(
                move || s.rows.get().is_empty(),
                || label(res::str::no_resources()).secondary(),
            ),
            list(
                items(
                    move || s.rows.get(),
                    |r: &day_webdav::Resource| r.href.clone(),
                ),
                move |slot| {
                    column((
                        label(move || slot.get().href).monospace().single_line(),
                        label(move || {
                            let resource = slot.get();
                            let statuses = resource
                                .status
                                .into_iter()
                                .chain(resource.properties.iter().map(|p| p.status))
                                .map(|s| s.to_string())
                                .collect::<Vec<_>>()
                                .join(", ");
                            format!("{statuses} · {}", resource.etag().unwrap_or_default())
                        })
                        .font(Font::Caption)
                        .secondary(),
                    ))
                    .spacing(Space::XS)
                    .padding(Space::S)
                },
            )
            .row_height(RowHeight::Uniform(64.0))
            .on_select(move |id| s.selected(id))
            .a11y(|a| a.label(res::str::resources()))
            .id(day::id!(resource_list))
            .height(220.0),
            section((
                labeled(
                    res::str::payload(),
                    text_field(s.payload)
                        .min_width(180.0)
                        .id(day::id!(upload_payload)),
                ),
                labeled(
                    res::str::etag(),
                    text_field(s.etag)
                        .min_width(180.0)
                        .id(day::id!(resource_etag)),
                ),
                labeled(
                    res::str::destination(),
                    text_field(s.destination)
                        .min_width(180.0)
                        .id(day::id!(destination_path)),
                ),
                row((
                    button(res::str::create_collection())
                        .enabled(move || s.writes.get())
                        .action(move || s.run(Operation::Mkcol))
                        .id(day::id!(create_collection)),
                    button(res::str::upload())
                        .enabled(move || s.writes.get())
                        .action(move || s.run(Operation::Put))
                        .id(day::id!(upload_resource)),
                    button(res::str::replace())
                        .enabled(move || s.writes.get())
                        .action(move || s.run(Operation::Replace))
                        .id(day::id!(replace_resource)),
                ))
                .spacing(Space::S)
                .fit(RowFit::Wrap { run_spacing: 8.0 }),
                row((
                    button(res::str::copy())
                        .enabled(move || s.writes.get())
                        .action(move || s.run(Operation::Copy))
                        .id(day::id!(copy_resource)),
                    button(res::str::move_resource())
                        .enabled(move || s.writes.get())
                        .action(move || s.run(Operation::Move))
                        .id(day::id!(move_resource)),
                    button(res::str::delete())
                        .enabled(move || s.writes.get())
                        .action(move || s.run(Operation::Delete))
                        .id(day::id!(delete_resource)),
                ))
                .spacing(Space::S)
                .fit(RowFit::Wrap { run_spacing: 8.0 }),
            )),
            row((
                button(res::str::slow())
                    .action(move || s.run(Operation::Slow))
                    .id(day::id!(slow_request)),
                button(res::str::broken())
                    .action(move || s.run(Operation::Broken))
                    .id(day::id!(broken_request)),
            ))
            .spacing(Space::S)
            .fit(RowFit::Wrap { run_spacing: 8.0 }),
            status(s),
        ))
        .spacing(Space::M)
        .padding(Space::L)
        .align(HAlign::Leading)
        .grow(Dimension::Width),
    )
}
pub fn response() -> impl Piece {
    let s = Scene::ambient();
    scroll(
        column((
            label(res::str::response_title())
                .font(Font::Title2)
                .bold()
                .id(day::id!(response_title)),
            label(res::str::response_help()).secondary(),
            label(res::str::request()).font(Font::Headline),
            label(s.request)
                .monospace()
                .selectable()
                .id(day::id!(request_details)),
            label(res::str::response()).font(Font::Headline),
            label(s.response)
                .monospace()
                .selectable()
                .id(day::id!(response_details)),
            label(res::str::statistics()).font(Font::Headline),
            label(s.statistics).id(day::id!(transfer_statistics)),
        ))
        .spacing(Space::M)
        .padding(Space::L)
        .align(HAlign::Leading)
        .grow(Dimension::Width),
    )
}

/// Files: sign in, then browse the account as a tree beside a preview of the selection.
pub fn files() -> impl Piece {
    let s = Scene::ambient();
    when(move || s.files.signed_in(), move || account(s.files)).otherwise(move || sign_in(s))
}
fn sign_in(s: Scene) -> impl Piece {
    let f = s.files;
    scroll(
        column((
            label(res::str::files_title())
                .font(Font::Title2)
                .bold()
                .id(day::id!(files_title)),
            label(res::str::files_help()).secondary(),
            section((
                labeled(
                    res::str::endpoint(),
                    text_field(s.endpoint)
                        .min_width(220.0)
                        .id(day::id!(files_endpoint)),
                ),
                labeled(
                    res::str::username(),
                    text_field(s.username)
                        .min_width(220.0)
                        .id(day::id!(files_username)),
                ),
                labeled(
                    res::str::files_password(),
                    secure_field(s.password)
                        .min_width(220.0)
                        .id(day::id!(files_password)),
                ),
            )),
            row((
                button(res::str::files_sign_in())
                    .prominent()
                    .enabled(move || !f.busy.get())
                    .action(move || s.sign_in())
                    .id(day::id!(files_sign_in)),
                button(res::str::files_fastmail())
                    .action(move || s.use_fastmail())
                    .id(day::id!(files_fastmail)),
                button(res::str::fixture())
                    .action(move || s.fixture())
                    .id(day::id!(files_fixture)),
            ))
            .spacing(Space::S)
            .fit(RowFit::Wrap { run_spacing: 8.0 }),
            label(f.status).id(day::id!(files_status)),
        ))
        .spacing(Space::L)
        .padding(Space::L)
        .align(HAlign::Leading)
        .grow(Dimension::Width),
    )
}
fn account(f: Files) -> impl Piece {
    let share = Signal::new(0.38);
    column((
        label(f.account)
            .font(Font::Headline)
            .single_line()
            .id(day::id!(files_account)),
        row((
            button(res::str::files_new_folder())
                .enabled(move || !f.busy.get())
                .action(move || f.create(true))
                .id(day::id!(files_new_folder)),
            button(res::str::files_new_file())
                .enabled(move || !f.busy.get())
                .action(move || f.create(false))
                .id(day::id!(files_new_file)),
            button(res::str::files_refresh())
                .enabled(move || !f.busy.get())
                .action(move || f.reload())
                .id(day::id!(files_refresh)),
            button(res::str::files_sign_out())
                .action(move || f.sign_out())
                .id(day::id!(files_sign_out)),
        ))
        .spacing(Space::S)
        .fit(RowFit::Wrap { run_spacing: 8.0 }),
        label(f.status)
            .font(Font::Footnote)
            .secondary()
            .id(day::id!(files_status)),
        // The size class picks a split with a fixed axis rather than feeding `.axis` a closure:
        // a composed split reads its axis while it holds the tree, and through day 0.6.3 a
        // closure that asked for the size class there panicked (WinUI, Android, ArkUI, web).
        switch(
            || day::size_class().is_some_and(|c| c.width == WidthClass::Compact),
            move |compact: &bool| {
                split(file_tree(f), preview(f))
                    .axis(if *compact {
                        SplitAxis::Vertical
                    } else {
                        SplitAxis::Horizontal
                    })
                    .fraction(share)
            },
        )
        .grow(Dimension::Both),
    ))
    .spacing(Space::S)
    .padding(Space::L)
    .align(HAlign::Leading)
}
fn file_tree(f: Files) -> impl Piece {
    tree(
        branches(
            move || f.nodes.get(),
            |e: &Entry| e.key.clone(),
            |e: &Entry| e.parent.clone(),
        ),
        |slot: ItemSlot<Entry, String>| {
            row((
                vector(move || {
                    if slot.field(|e| e.collection) {
                        res::vectors::folder
                    } else {
                        res::vectors::file
                    }
                })
                .template()
                .decorative()
                .width(16.0)
                .height(16.0),
                label(move || slot.field(|e| e.name.clone())).single_line(),
                spacer(),
                label(move || {
                    slot.field(|e| {
                        e.size
                            .filter(|_| !e.collection)
                            .map(size_text)
                            .unwrap_or_default()
                    })
                })
                .font(Font::Caption)
                .secondary()
                .single_line(),
            ))
            .spacing(Space::S)
            .padding(Insets::symmetric(4.0, 0.0))
        },
    )
    .row_height(RowHeight::Uniform(28.0))
    .expanded(f.expanded)
    .expandable(|key: &String| key.ends_with('/'))
    .on_expansion(move |key: String, open| f.expand(key, open))
    .selected(move || f.selection.get())
    .on_selection(move |keys: Vec<String>| f.select(keys.into_iter().next()))
    .row_id(|key: &String| day::id!(file_node, key.as_str()))
    .a11y(|a| a.label(res::str::files_tree()))
    .id(day::id!(file_tree))
}
/// Size, type and date for the selection's caption.
fn summary(f: Files) -> String {
    let Some(entry) = f.selected_entry() else {
        return String::new();
    };
    if entry.collection {
        return res::str::files_folder().format();
    }
    let content_type = f.content_type.get();
    [
        entry.size.map(size_text),
        Some(
            content_type
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned(),
        ),
        entry.modified,
    ]
    .into_iter()
    .flatten()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}
fn preview(f: Files) -> impl Piece {
    let file = move || f.selected_entry().is_some_and(|e| !e.collection);
    scroll(
        column((
            when(
                move || f.selected_entry().is_none(),
                || label(res::str::files_select()).secondary(),
            ),
            when(
                move || f.selected_entry().is_some(),
                move || {
                    column((
                        label(move || f.selected_entry().map(|e| e.name).unwrap_or_default())
                            .font(Font::Title3)
                            .bold()
                            .selectable()
                            .id(day::id!(preview_name)),
                        label(move || summary(f))
                            .font(Font::Caption)
                            .secondary()
                            .id(day::id!(preview_details)),
                        row((
                            when(
                                move || f.is_text.get(),
                                move || {
                                    row((
                                        button(res::str::files_save())
                                            .prominent()
                                            .enabled(move || f.dirty() && !f.busy.get())
                                            .action(move || f.save())
                                            .id(day::id!(preview_save)),
                                        button(res::str::files_revert())
                                            .enabled(move || f.dirty())
                                            .action(move || f.revert())
                                            .id(day::id!(preview_revert)),
                                    ))
                                    .spacing(Space::S)
                                },
                            ),
                            when(file, move || {
                                button(res::str::files_replace())
                                    .enabled(move || !f.busy.get())
                                    .action(move || f.replace())
                                    .id(day::id!(preview_replace))
                            }),
                            button(res::str::files_rename_action())
                                .enabled(move || !f.busy.get())
                                .action(move || f.rename())
                                .id(day::id!(preview_rename)),
                            button(res::str::files_delete_action())
                                .enabled(move || !f.busy.get())
                                .action(move || f.delete())
                                .id(day::id!(preview_delete)),
                        ))
                        .spacing(Space::S)
                        .fit(RowFit::Wrap { run_spacing: 8.0 }),
                        switch(
                            move || f.preview.get(),
                            move |p: &Preview| match p {
                                Preview::Loading => {
                                    row((spinner(), label(res::str::files_loading()).secondary()))
                                        .spacing(Space::S)
                                        .any()
                                }
                                Preview::Folder => label(move || {
                                    let key = f.current().unwrap_or_default();
                                    let count = f.nodes.with(|n| {
                                        n.iter()
                                            .filter(|e| e.parent.as_deref() == Some(&key))
                                            .count()
                                    });
                                    if f.loaded.with(|l| l.contains(&key)) {
                                        res::str::files_folder_items(count as i64).format()
                                    } else {
                                        res::str::files_loading().format()
                                    }
                                })
                                .secondary()
                                .id(day::id!(preview_folder))
                                .any(),
                                Preview::Image => image(move || {
                                    f.image
                                        .get()
                                        .map(day::ImageSource::Bytes)
                                        .unwrap_or_default()
                                })
                                .fit()
                                .height(320.0)
                                .id(day::id!(preview_image))
                                .any(),
                                Preview::Binary => label(res::str::files_no_preview())
                                    .secondary()
                                    .id(day::id!(preview_binary))
                                    .any(),
                                Preview::Failed(message) => {
                                    label(message.clone()).id(day::id!(preview_error)).any()
                                }
                                Preview::Nothing | Preview::Text => spacer().height(0.0).any(),
                            },
                        ),
                        when(
                            move || f.is_text.get(),
                            move || {
                                text_area(f.text)
                                    .min_lines(14)
                                    .spellcheck(false)
                                    .a11y(|a| a.label(res::str::files_contents()))
                                    .id(day::id!(preview_text))
                            },
                        ),
                    ))
                    .spacing(Space::M)
                    .align(HAlign::Leading)
                    .grow(Dimension::Width)
                },
            ),
        ))
        .spacing(Space::M)
        .padding(Space::M)
        .align(HAlign::Leading)
        .grow(Dimension::Width),
    )
}
