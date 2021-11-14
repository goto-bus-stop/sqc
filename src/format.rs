//! Format SQL strings with highlighting.
//!
//! Falls back to `sqlformat` if a thing is not yet implemented.
use colored::Colorize;
use sqlparser::ast::{
    ColumnOption, ColumnOptionDef, Ident, ObjectName, Statement, TableConstraint,
};
use std::fmt::Write;

fn format_sql_object_name(
    buf: &mut String,
    ObjectName(object_name): ObjectName,
) -> std::fmt::Result {
    for (index, ident) in object_name.into_iter().enumerate() {
        if index > 0 {
            write!(buf, ".")?;
        }
        if let Some(quote) = ident.quote_style {
            write!(buf, "{}{}{}", quote, ident.value, quote)?;
        } else {
            write!(buf, "{}", ident.value)?;
        }
    }
    Ok(())
}

fn format_sql_column_option(buf: &mut String, option: ColumnOptionDef) -> std::fmt::Result {
    use ColumnOption::*;
    match option.option {
        Null => write!(buf, "{}", "NULL".blue().bold()),
        NotNull => write!(buf, "{}", "NOT NULL".blue().bold()),
        Unique { is_primary } => write!(
            buf,
            "{}",
            if is_primary {
                "PRIMARY KEY"
            } else {
                "UNIQUE"
            }
            .blue()
            .bold()
        ),
        Default(expr) => write!(buf, "{} {}", "DEFAULT".blue().bold(), expr),
        ForeignKey {
            foreign_table,
            referred_columns,
            on_delete,
            on_update,
        } => {
            let mut name = String::new();
            format_sql_object_name(&mut name, foreign_table)?;
            write!(
                buf,
                "{} {}({})",
                "REFERENCES".blue().bold(),
                name,
                referred_columns[0]
            )?;
            if let Some(action) = on_delete {
                write!(
                    buf,
                    " {} {}",
                    "ON DELETE".blue().bold(),
                    action.to_string().blue().bold()
                )?;
            }
            if let Some(action) = on_update {
                write!(
                    buf,
                    " {} {}",
                    "ON UPDATE".blue().bold(),
                    action.to_string().blue().bold()
                )?;
            }
            Ok(())
        }
        _ => todo!(),
    }
}

fn format_sql_table_constraint(buf: &mut String, constraint: TableConstraint) -> std::fmt::Result {
    use TableConstraint::*;
    let mut format_name = |name, ty: &str, columns: Vec<Ident>| {
        if let Some(name) = name {
            write!(buf, "{} {} ", "CONSTRAINT".blue().bold(), name)?;
        }
        write!(buf, "{} (", ty.blue().bold())?;

        for (index, column) in columns.into_iter().enumerate() {
            if index > 0 {
                write!(buf, ", ")?;
            }
            write!(buf, "{}", column)?;
        }

        write!(buf, ")")
    };

    match constraint {
        Unique {
            name,
            columns,
            is_primary,
        } => format_name(
            name,
            if is_primary {
                "PRIMARY KEY"
            } else {
                "UNIQUE"
            },
            columns,
        ),
        ForeignKey {
            name,
            columns,
            foreign_table,
            referred_columns,
            on_delete,
            on_update,
        } => {
            format_name(name, "FOREIGN KEY", columns)?;

            let mut name = String::new();
            format_sql_object_name(&mut name, foreign_table)?;
            write!(
                buf,
                "{} {}({})",
                "REFERENCES".blue().bold(),
                name,
                referred_columns[0]
            )?;
            if let Some(action) = on_delete {
                write!(
                    buf,
                    " {} {}",
                    "ON DELETE".blue().bold(),
                    action.to_string().blue().bold()
                )?;
            }
            if let Some(action) = on_update {
                write!(
                    buf,
                    " {} {}",
                    "ON UPDATE".blue().bold(),
                    action.to_string().blue().bold()
                )?;
            }
            Ok(())
        }
        _ => todo!(),
    }
}

pub fn format_sql_statement(stmt: Statement) -> anyhow::Result<String> {
    let mut buf = String::new();

    use Statement::*;
    match stmt {
        CreateTable {
            if_not_exists,
            name,
            columns,
            constraints,
            without_rowid,
            ..
        } => {
            write!(&mut buf, "{} ", "CREATE TABLE".blue().bold())?;
            if if_not_exists {
                write!(&mut buf, "{} ", "IF NOT EXISTS".blue().bold())?;
            }
            format_sql_object_name(&mut buf, name)?;
            writeln!(&mut buf, " (")?;

            for column in columns {
                write!(
                    &mut buf,
                    "  {} {}",
                    column.name,
                    column.data_type.to_string().blue().bold()
                )?;
                for option in column.options {
                    write!(&mut buf, " ")?;
                    format_sql_column_option(&mut buf, option)?;
                }
                writeln!(&mut buf, ",")?;
            }

            for constraint in constraints {
                write!(&mut buf, "  ")?;
                format_sql_table_constraint(&mut buf, constraint)?;
                writeln!(&mut buf, ",")?;
            }

            write!(&mut buf, ")")?;

            if without_rowid {
                write!(&mut buf, " {}", "WITHOUT ROWID".blue().bold())?;
            }
        }
        _ => {
            let formatted =
                sqlformat::format(&stmt.to_string(), &Default::default(), Default::default());
            write!(&mut buf, "{}", formatted)?;
        }
    }
    Ok(buf)
}
