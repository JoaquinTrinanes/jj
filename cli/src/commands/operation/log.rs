// Copyright 2020-2023 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::slice;

use jj_lib::op_walk;
use jj_lib::operation::Operation;
use jj_lib::settings::ConfigResultExt as _;
use jj_lib::settings::UserSettings;

use crate::cli_util::format_template;
use crate::cli_util::CommandHelper;
use crate::cli_util::LogContentFormat;
use crate::cli_util::WorkspaceCommandEnvironment;
use crate::command_error::CommandError;
use crate::graphlog::get_graphlog;
use crate::graphlog::Edge;
use crate::graphlog::GraphStyle;
use crate::operation_templater::OperationTemplateLanguage;
use crate::ui::Ui;

/// Show the operation log
///
/// Like other commands, `jj op log` snapshots the current working-copy changes
/// and reconciles divergent operations. Use `--at-op=@ --ignore-working-copy`
/// to inspect the current state without mutation.
#[derive(clap::Args, Clone, Debug)]
pub struct OperationLogArgs {
    /// Limit number of operations to show
    #[arg(long, short = 'n')]
    limit: Option<usize>,
    // TODO: Delete `-l` alias in jj 0.25+
    #[arg(
        short = 'l',
        hide = true,
        conflicts_with = "limit",
        value_name = "LIMIT"
    )]
    deprecated_limit: Option<usize>,
    /// Don't show the graph, show a flat list of operations
    #[arg(long)]
    no_graph: bool,
    /// Render each operation using the given template
    ///
    /// For the syntax, see https://martinvonz.github.io/jj/latest/templates/
    #[arg(long, short = 'T')]
    template: Option<String>,
}

pub fn cmd_op_log(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &OperationLogArgs,
) -> Result<(), CommandError> {
    if command.is_working_copy_writable() {
        let workspace_command = command.workspace_helper(ui)?;
        let current_op = workspace_command.repo().operation();
        do_op_log(ui, workspace_command.env(), current_op, args)
    } else {
        // Don't load the repo so that the operation history can be inspected
        // even with a corrupted repo state. For example, you can find the first
        // bad operation id to be abandoned.
        let workspace = command.load_workspace()?;
        let workspace_env = command.workspace_environment(ui, &workspace)?;
        let current_op = command.resolve_operation(ui, workspace.repo_loader())?;
        do_op_log(ui, &workspace_env, &current_op, args)
    }
}

fn do_op_log(
    ui: &mut Ui,
    workspace_env: &WorkspaceCommandEnvironment,
    current_op: &Operation,
    args: &OperationLogArgs,
) -> Result<(), CommandError> {
    let settings = workspace_env.settings();
    let op_store = current_op.op_store();

    let graph_style = GraphStyle::from_settings(settings)?;
    let with_content_format = LogContentFormat::new(ui, settings)?;

    let template;
    let op_node_template;
    {
        let language = OperationTemplateLanguage::new(
            op_store.root_operation_id(),
            Some(current_op.id()),
            workspace_env.operation_template_extensions(),
        );
        let text = match &args.template {
            Some(value) => value.to_owned(),
            None => settings.config().get_string("templates.op_log")?,
        };
        template = workspace_env
            .parse_template(&language, &text, OperationTemplateLanguage::wrap_operation)?
            .labeled("op_log");
        op_node_template = workspace_env
            .parse_template(
                &language,
                &get_node_template(graph_style, settings)?,
                OperationTemplateLanguage::wrap_operation,
            )?
            .labeled("node");
    }

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    let formatter = formatter.as_mut();
    if args.deprecated_limit.is_some() {
        writeln!(
            ui.warning_default(),
            "The -l shorthand is deprecated, use -n instead."
        )?;
    }
    let limit = args.limit.or(args.deprecated_limit).unwrap_or(usize::MAX);
    let iter = op_walk::walk_ancestors(slice::from_ref(current_op)).take(limit);
    if !args.no_graph {
        let mut graph = get_graphlog(graph_style, formatter.raw());
        for op in iter {
            let op = op?;
            let mut edges = vec![];
            for id in op.parent_ids() {
                edges.push(Edge::Direct(id.clone()));
            }
            let mut buffer = vec![];
            let within_graph = with_content_format.sub_width(graph.width(op.id(), &edges));
            within_graph.write(ui.new_formatter(&mut buffer).as_mut(), |formatter| {
                template.format(&op, formatter)
            })?;
            if !buffer.ends_with(b"\n") {
                buffer.push(b'\n');
            }
            let node_symbol = format_template(ui, &op, &op_node_template);
            graph.add_node(
                op.id(),
                &edges,
                &node_symbol,
                &String::from_utf8_lossy(&buffer),
            )?;
        }
    } else {
        for op in iter {
            let op = op?;
            with_content_format.write(formatter, |formatter| template.format(&op, formatter))?;
        }
    }

    Ok(())
}

fn get_node_template(
    style: GraphStyle,
    settings: &UserSettings,
) -> Result<String, config::ConfigError> {
    let symbol = settings
        .config()
        .get_string("templates.op_log_node")
        .optional()?;
    let default = if style.is_ascii() {
        "builtin_op_log_node_ascii"
    } else {
        "builtin_op_log_node"
    };
    Ok(symbol.unwrap_or_else(|| default.to_owned()))
}