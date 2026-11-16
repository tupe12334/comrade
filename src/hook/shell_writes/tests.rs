use super::write_targets;

fn targets(cmd: &str) -> Vec<String> {
    write_targets(cmd)
}

#[test]
fn detects_output_redirect() {
    assert_eq!(targets("echo hi > src/main.rs"), vec!["src/main.rs"]);
    assert_eq!(targets("echo hi >> a.py"), vec!["a.py"]);
    assert_eq!(targets("echo hi >| b.go"), vec!["b.go"]);
    assert_eq!(targets("make && echo x > out.sh"), vec!["out.sh"]);
}

#[test]
fn ignores_fd_dups_and_null() {
    assert!(targets("cargo build 2>&1").is_empty());
    assert!(targets("echo x > /dev/null").is_empty());
    assert!(targets("cmd 2>/dev/null").is_empty());
}

#[test]
fn detects_combined_stdout_stderr_redirect() {
    assert_eq!(targets("cmd &> all.log"), vec!["all.log"]);
}

#[test]
fn heredoc_body_is_not_a_write() {
    let cmd = "cat > lib.rs <<'EOF'\nlet x = 1; // > fake.rs\nEOF\n";
    assert_eq!(targets(cmd), vec!["lib.rs"]);
}

#[test]
fn quoted_redirect_is_text() {
    assert!(targets("echo \"a > b.rs\"").is_empty());
    assert!(targets("echo 'x >> y.ts'").is_empty());
}

#[test]
fn input_redirect_is_not_a_write() {
    assert!(targets("wc -l < notes.rs").is_empty());
}

#[test]
fn detects_tee() {
    assert_eq!(
        targets("cargo build 2>&1 | tee build.log"),
        vec!["build.log"]
    );
    assert_eq!(targets("echo x | tee -a a.ts b.ts"), vec!["a.ts", "b.ts"]);
}

#[test]
fn detects_sed_in_place() {
    assert_eq!(targets("sed -i 's/a/b/' x.ts"), vec!["x.ts"]);
    assert_eq!(targets("sed -i.bak 's/a/b/' x.ts"), vec!["x.ts"]);
    assert_eq!(
        targets("sed -i '' -e 's/a/b/' y.go z.go"),
        vec!["y.go", "z.go"]
    );
    assert_eq!(targets("sed -E -i 's/a/b/' w.c"), vec!["w.c"]);
}

#[test]
fn sed_without_in_place_writes_nothing() {
    assert!(targets("sed 's/a/b/' x.ts").is_empty());
    assert!(targets("sed -n 's/a/b/p' x.ts").is_empty());
}

#[test]
fn detects_cp_mv_and_dd() {
    assert_eq!(targets("cp a.rs b.rs"), vec!["b.rs"]);
    assert_eq!(targets("mv -f tmp.py final.py"), vec!["final.py"]);
    assert_eq!(targets("dd if=x of=y.c"), vec!["y.c"]);
}

#[test]
fn plain_commands_write_nothing() {
    assert!(targets("ls -la").is_empty());
    assert!(targets("cargo test").is_empty());
}

#[test]
fn quoted_angle_bracket_does_not_hide_later_write() {
    assert_eq!(
        targets("echo \"a<<X\"\necho hi > src/main.rs"),
        vec!["src/main.rs"]
    );
}

#[test]
fn ansi_c_quoted_target_is_unquoted() {
    assert_eq!(targets("echo $'it\\'s' > src/main.rs"), vec!["src/main.rs"]);
}

#[test]
fn multiple_heredocs_on_one_line() {
    assert_eq!(
        targets("cat <<A <<B\na\nA\nb > nope.rs\nB\necho > real.rs"),
        vec!["real.rs"]
    );
}

#[test]
fn read_write_fd_open_is_a_write() {
    assert_eq!(targets("exec 3<>src/main.rs"), vec!["src/main.rs"]);
}

#[test]
fn writes_inside_compound_commands_are_found() {
    assert_eq!(targets("if true; then echo a > x.rs; fi"), vec!["x.rs"]);
    assert_eq!(targets("for i in 1; do echo $i; done > y.rs"), vec!["y.rs"]);
    assert_eq!(targets("f() { echo a > z.rs; }; f"), vec!["z.rs"]);
}

#[test]
fn parse_error_falls_back_to_whitespace_scan() {
    assert_eq!(targets("echo a > b.rs ) )"), vec!["b.rs"]);
}

#[test]
fn parse_error_keeps_cp_and_tee_checks() {
    assert_eq!(targets("cp /tmp/x src/main.rs\n)"), vec!["src/main.rs"]);
    assert_eq!(targets("echo hi | tee src/main.rs\n)"), vec!["src/main.rs"]);
}

#[test]
fn parse_error_keeps_sed_mv_install_dd_checks() {
    assert_eq!(targets("sed -i 's/a/b/' src/a.rs\n)"), vec!["src/a.rs"]);
    assert_eq!(targets("mv tmp.rs src/b.rs\n)"), vec!["src/b.rs"]);
    assert_eq!(targets("install x src/c.rs\n)"), vec!["src/c.rs"]);
    assert_eq!(targets("dd if=x of=src/d.rs\n)"), vec!["src/d.rs"]);
}

#[test]
fn parse_error_fallback_removes_quotes() {
    assert_eq!(targets("echo x > 'src/main.rs'\n)"), vec!["src/main.rs"]);
    assert_eq!(targets("echo x > \"src/main.rs\"\n)"), vec!["src/main.rs"]);
}

#[test]
fn parse_error_fallback_still_reads_fd_and_read_only_commands() {
    assert!(targets("cat src/main.rs\n)").is_empty());
    assert!(targets("cargo build 2>&1\n)").is_empty());
}

#[test]
fn detects_writes_inside_dollar_paren_substitution() {
    assert_eq!(
        targets("echo $(cp /tmp/x src/main.rs)"),
        vec!["src/main.rs"]
    );
    assert_eq!(
        targets("echo \"$(cp /tmp/x src/main.rs)\""),
        vec!["src/main.rs"]
    );
}

#[test]
fn detects_writes_inside_assignment_substitution() {
    assert_eq!(targets("x=$(tee src/main.rs <<<code)"), vec!["src/main.rs"]);
}

#[test]
fn detects_writes_inside_backtick_substitution() {
    assert_eq!(targets("echo `cp /tmp/x src/main.rs`"), vec!["src/main.rs"]);
}

#[test]
fn single_quoted_substitution_is_text() {
    assert!(targets("echo '$(cp /tmp/x src/main.rs)'").is_empty());
    assert!(targets("echo \"$(cat src/main.rs)\"").is_empty());
}

#[test]
fn deeply_nested_parens_still_find_the_write() {
    let depth = 500;
    let cmd = format!(
        "{}echo x > src/main.rs{}",
        "(".repeat(depth),
        ")".repeat(depth)
    );
    assert_eq!(targets(&cmd), vec!["src/main.rs"]);
}

#[test]
fn fifty_thousand_nested_parens_do_not_overflow() {
    let depth = 50_000;
    let cmd = format!(
        "{}echo x > src/main.rs{}",
        "(".repeat(depth),
        ")".repeat(depth)
    );
    assert_eq!(targets(&cmd), vec!["src/main.rs"]);
}

#[test]
fn duplicate_output_to_a_file_is_a_write() {
    assert_eq!(targets("echo x >&src/main.rs"), vec!["src/main.rs"]);
    assert_eq!(targets("echo x >& src/main.rs"), vec!["src/main.rs"]);
    assert!(targets("echo x >&2").is_empty());
    assert!(targets("echo x 3>&1 1>&2 2>&3-").is_empty());
    assert!(targets("exec 3>&-").is_empty());
}

#[test]
fn parse_error_fallback_finds_duplicate_output_to_a_file() {
    assert_eq!(targets("echo x >&src/main.rs\n)"), vec!["src/main.rs"]);
    assert_eq!(targets("echo x >& src/main.rs\n)"), vec!["src/main.rs"]);
    assert!(targets("cargo build >&2\n)").is_empty());
}

#[test]
fn ansi_c_escapes_are_decoded_like_bash() {
    assert_eq!(targets("echo x > $'\\x73rc/main.rs'"), vec!["src/main.rs"]);
    assert_eq!(targets("echo x > $'\\163rc/main.rs'"), vec!["src/main.rs"]);
    assert_eq!(
        targets("echo x > $'\\u0073rc/main.rs'"),
        vec!["src/main.rs"]
    );
    assert_eq!(
        targets("echo x > $'src/main.rs\\x00.txt'"),
        vec!["src/main.rs"]
    );
}

#[test]
fn parse_error_fallback_decodes_ansi_c_escapes() {
    assert_eq!(
        targets("echo x > $'\\x73rc/main.rs'\n)"),
        vec!["src/main.rs"]
    );
}

#[test]
fn quoted_or_escaped_paren_does_not_end_a_substitution() {
    assert_eq!(
        targets("echo $(echo \")\" > src/main.rs)"),
        vec!["src/main.rs"]
    );
    assert_eq!(
        targets("echo $(echo \")\"; cp /tmp/x src/main.rs)"),
        vec!["src/main.rs"]
    );
    assert_eq!(
        targets("echo $(echo \\); cp /tmp/x src/main.rs)"),
        vec!["src/main.rs"]
    );
}

#[test]
fn case_pattern_paren_in_a_substitution_still_finds_the_write() {
    assert_eq!(
        targets("echo $(case x in x) cp /tmp/x src/main.rs;; esac)"),
        vec!["src/main.rs"]
    );
}

#[test]
fn apostrophe_in_heredoc_or_comment_does_not_hide_a_substitution() {
    assert_eq!(
        targets("cat <<EOF\nit's\nEOF\necho $(cp /tmp/x src/main.rs)"),
        vec!["src/main.rs"]
    );
    assert_eq!(
        targets("# don't\necho $(cp /tmp/x src/main.rs)"),
        vec!["src/main.rs"]
    );
}

#[test]
fn substitutions_in_heredocs_and_expansions_are_found() {
    let found = vec!["src/main.rs"];
    assert_eq!(targets("cat <<EOF\n$(cp /tmp/x src/main.rs)\nEOF"), found);
    assert!(targets("cat <<'EOF'\n$(cp /tmp/x src/main.rs)\nEOF").is_empty());
    assert_eq!(targets("echo ${x:-$(cp /tmp/x src/main.rs)}"), found);
    assert_eq!(targets("echo \"${x:-$(cp /tmp/x src/main.rs)}\""), found);
    assert_eq!(targets("echo $(( $(cp /tmp/x src/main.rs) ))"), found);
    assert_eq!(
        targets("for f in $(cp /tmp/x src/main.rs); do :; done"),
        found
    );
    assert_eq!(targets("[[ -n $(cp /tmp/x src/main.rs) ]]"), found);
}

#[test]
fn mixed_ansi_c_quoting_in_a_target_is_decoded() {
    assert_eq!(targets("echo x > src/$'main.rs'"), vec!["src/main.rs"]);
    assert_eq!(targets("echo x > 'src/'$'main.rs'"), vec!["src/main.rs"]);
    assert_eq!(targets("echo x > \"src/\"$'main'.rs"), vec!["src/main.rs"]);
    assert_eq!(targets("cp /tmp/x src/$'\\x6d'ain.rs"), vec!["src/main.rs"]);
}

#[test]
fn parse_error_fallback_keeps_separators_in_double_quotes() {
    assert_eq!(targets("cp \"a;b\" src/main.rs\n)"), vec!["src/main.rs"]);
}

#[test]
fn parse_error_fallback_skips_heredoc_body_with_apostrophe() {
    let cmd = "cat <<'EOF'\ndon't\nEOF\necho x > src/main.rs\n)";
    assert_eq!(targets(cmd), vec!["src/main.rs"]);
}

#[test]
fn parse_error_fallback_reads_double_quoted_redirect_as_text() {
    assert!(targets("echo \"a > b.rs\"\n)").is_empty());
}

#[test]
fn heredoc_body_with_many_openers_is_not_a_write() {
    let body = "f(x) => y.rs\n".repeat(70);
    let cmd = format!("cat > notes.md <<'EOF'\n{body}EOF\n");
    assert_eq!(targets(&cmd), vec!["notes.md"]);
}

#[test]
fn parse_error_fallback_reads_substitution_in_double_quotes() {
    assert_eq!(targets("echo \"$(echo x > a.rs)\"\n)"), vec!["a.rs"]);
    let nested_case = "echo \"$(case x in a) echo > f.rs;; esac)\"\n)";
    assert_eq!(targets(nested_case), vec!["f.rs"]);
}

#[test]
fn parse_error_fallback_reads_substitution_in_expanding_heredoc_body() {
    assert_eq!(targets("cat <<EOF\n$(echo x > a.rs)\nEOF\n)"), vec!["a.rs"]);
    assert_eq!(targets("cat <<-EOF\n\t`tee b.rs`\n\tEOF\n)"), vec!["b.rs"]);
}

#[test]
fn parse_error_fallback_skips_substitution_in_quoted_heredoc_body() {
    assert!(targets("cat <<'EOF'\n$(echo x > a.rs)\nEOF\n)").is_empty());
    assert!(targets("cat <<\\EOF\n`tee b.rs`\nEOF\n)").is_empty());
}

#[test]
fn parse_error_fallback_ends_heredoc_on_delimiter_with_space() {
    let cmd = "cat <<'E '\nx\nE \necho hi > src/main.rs\n)";
    assert_eq!(targets(cmd), vec!["src/main.rs"]);
}

#[test]
fn nested_subshells_read_as_arithmetic_still_find_writes() {
    // brush-parser reads `( (` as an arithmetic command `((...))`.
    assert_eq!(targets("( (echo x > src/main.rs) )"), vec!["src/main.rs"]);
    assert_eq!(targets("((echo x > a.rs))"), vec!["a.rs"]);
    assert_eq!(targets("( ( cp a b.rs ) )"), vec!["b.rs"]);
    assert_eq!(targets("( ( (echo x > c.rs) ) )"), vec!["c.rs"]);
    assert_eq!(targets("( ( ( ( tee d.rs ) ) ) )"), vec!["d.rs"]);
    assert_eq!(targets("echo $( (echo x > e.rs) )"), vec!["e.rs"]);
    assert_eq!(targets("echo \"$( ( (echo x > n.rs) ) )\""), vec!["n.rs"]);
    assert_eq!(targets("((x=1)); echo y > a.rs"), vec!["a.rs"]);
}

#[test]
fn plain_arithmetic_is_not_a_write() {
    assert!(targets("((i++))").is_empty());
    assert!(targets("((i++)); echo ok").is_empty());
    assert!(targets("for ((i=0;i<3;i++)); do echo; done").is_empty());
    assert!(targets("echo $((1+2))").is_empty());
}
