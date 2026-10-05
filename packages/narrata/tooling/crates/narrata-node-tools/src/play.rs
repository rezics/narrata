//! Playing a pack from the command line. Actions are option aliases; with a local content
//! pack the text is resolved and printed, otherwise the text-free book view is printed.

use std::sync::Arc;

use narrata_content_local::{
    Content, LocalContent, Payload, Resolution, ResolveContext, ResolveItem, ResolveRequest,
};
use narrata_kernel::content::ContentRef;
use narrata_nodes::{
    BookView, Error, NameTable, OptionId, Result, Session, ViewScalar, analyze,
    view::{Interaction, PresentationItem, Presented, Role},
};

/// Applies comma-separated steps. A step is option aliases joined by `+`; `~` chooses none.
pub fn act(session: &mut Session, names: Option<&NameTable>, actions: &str) -> Result<()> {
    let names = names.ok_or_else(|| {
        Error::new(
            "names",
            "actions",
            "actions by alias need a pack with its name table",
        )
    })?;
    for step in actions.split(',').filter(|step| !step.is_empty()) {
        // A hint may fail for an unselected route; the chosen route is checked by choose.
        let _ = session.prefetch();
        let view = session.view(Some(names))?;
        let Interaction::Choose {
            choice_point,
            options,
            ..
        } = view.interaction
        else {
            return Err(Error::new("finished", step, "the story has ended"));
        };
        let chosen = step
            .split('+')
            .filter(|key| *key != "~")
            .map(|key| {
                options
                    .iter()
                    .find(|option| option.key.as_deref() == Some(key))
                    .map(|option| option.id)
                    .ok_or_else(|| Error::new("action", step, format!("no option {key} here")))
            })
            .collect::<Result<Vec<OptionId>>>()?;
        session.choose(&view.cursor, choice_point, chosen)?;
    }
    Ok(())
}

pub fn book(session: &Session, names: Option<&NameTable>) -> Result<BookView> {
    let analysis = analyze(session.program(), names)?;
    Ok(BookView {
        view: session.view(names)?,
        page: session.page()?,
        graphs: analysis.graphs,
        diagnostics: analysis.diagnostics,
    })
}

struct Resolver<'a> {
    content: &'a LocalContent,
    languages: Vec<String>,
}

impl Resolver<'_> {
    fn resolve(
        &self,
        content: Content,
        args: &std::collections::BTreeMap<String, ViewScalar>,
    ) -> Result<String> {
        let request = ResolveRequest {
            context: ResolveContext {
                languages: self.languages.clone(),
                ..ResolveContext::default()
            },
            items: vec![ResolveItem {
                content: content.clone(),
                args: args.clone(),
            }],
        };
        let resolution = self.content.resolve(&request)?.into_iter().next();
        Ok(match resolution {
            Some(Resolution::Ok { payload, .. }) => match payload {
                Payload::Text { text } => text,
                Payload::Blocks { blocks } => blocks
                    .into_iter()
                    .map(|block| block.text)
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            },
            Some(Resolution::Incompatible { reason }) => format!("[incompatible: {reason}]"),
            _ => format!("[unavailable: {}]", label(&content)),
        })
    }

    fn reference(
        &self,
        reference: &ContentRef,
        args: &std::collections::BTreeMap<String, ViewScalar>,
    ) -> Result<String> {
        self.resolve(Content::Ref(reference.clone()), args)
    }
}

fn label(content: &Content) -> String {
    match content {
        Content::Ref(reference) => format!("{}:{}", reference.provider, reference.key),
        Content::Segment(segment) => format!("{}:{}", segment.unit.provider, segment.unit.key),
    }
}

/// The page and the interaction as plain text.
pub fn render(
    session: &Session,
    names: Option<&NameTable>,
    content: &LocalContent,
    languages: Vec<String>,
) -> Result<String> {
    let resolver = Resolver { content, languages };
    let view = session.view(names)?;
    let mut lines = vec![
        format!("artifact: {}", view.artifact_id),
        format!("commit: {}", view.cursor),
    ];
    let item = |item: &PresentationItem| -> Result<String> {
        let content = match &item.content {
            Presented::Ref(reference) => Content::Ref(reference.clone()),
            Presented::Segment(segment) => Content::Segment(segment.clone()),
        };
        let text = resolver.resolve(content, &item.args)?;
        Ok(match item.role {
            Role::Title => format!("# {text}"),
            Role::Body => text,
            Role::Reply => text
                .lines()
                .map(|line| {
                    if line.is_empty() {
                        ">".to_owned()
                    } else {
                        format!("> {line}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
        })
    };
    for presented in session.page()? {
        lines.push(String::new());
        lines.push(item(&presented)?);
    }
    lines.push(String::new());
    match &view.interaction {
        Interaction::Choose {
            min,
            max,
            args,
            options,
            ..
        } => {
            if (*min, *max) != (1, 1) {
                lines.push(format!("choose {min}..{max}:"));
            }
            for option in options {
                let key = option.key.as_deref().unwrap_or("?");
                let text = match &option.label {
                    Some(label) => resolver.reference(label, args)?,
                    None => String::new(),
                };
                let mut line = format!("* [{key}] {text}");
                if !option.enabled {
                    let reason = match &option.reason {
                        Some(reason) => resolver.reference(reason, args)?,
                        None => String::new(),
                    };
                    line.push_str(&format!(" (disabled: {reason})"));
                }
                lines.push(line);
            }
        }
        Interaction::Finished {
            outcome,
            title,
            body,
        } => {
            if let Some(title) = title {
                lines.push(format!(
                    "# {}",
                    resolver.reference(title, &Default::default())?
                ));
                lines.push(String::new());
            }
            if let Some(body) = body {
                lines.push(resolver.resolve(Content::Segment(body.clone()), &Default::default())?);
                lines.push(String::new());
            }
            lines.push(format!("(outcome: {outcome})"));
        }
    }
    Ok(lines.join("\n"))
}

pub fn new_session(
    program: Arc<narrata_nodes::Program>,
    execution: Option<&str>,
) -> Result<Session> {
    let execution = match execution {
        Some(text) => text.parse().map_err(|_| {
            Error::new(
                "identifier",
                "--execution",
                "expected execution:<32 hex digits>",
            )
        })?,
        None => crate::ids::execution_id()?,
    };
    Session::new(program, execution)
}
