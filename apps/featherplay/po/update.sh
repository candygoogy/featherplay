#!/usr/bin/env bash
# Regenerates the translation template, po/featherplay.pot, from the sources, and updates the
# languages from it. Run it after adding, changing or removing a translatable string:
#   apps/featherplay/po/update.sh
#
# 1. The strings Rust marks with tr!(…) and trn!(…), in src/ and in the crates, are extracted
#    here with Perl (Git for Windows ships it).
# 2. The @tr(…) strings in ui/ are added by slint-tr-extractor, at the Slint version the app
#    uses: `cargo install slint-tr-extractor --version 1.18.1` (or point SLINT_TR_EXTRACTOR at
#    it). No default context, as build.rs compiles them.
# 3. Each po/<lang>/LC_MESSAGES/featherplay.po is merged with the new template by msgmerge
#    (GNU gettext) when it's installed. Without it the script says so, and a translation tool
#    does the same: Poedit's Translation > Update from POT File, for one.
# 4. The test language, po/qps-ploc, is regenerated: every string in brackets.
#
# The test `translations::the_pot_holds_every_marked_string` fails while the template is out
# of date.

set -euo pipefail
shopt -s nullglob
cd "$(dirname "$0")/.."

extractor=${SLINT_TR_EXTRACTOR:-slint-tr-extractor}
if ! command -v "$extractor" >/dev/null; then
    echo "update.sh: slint-tr-extractor not found." >&2
    echo "  cargo install slint-tr-extractor --version 1.18.1, or set SLINT_TR_EXTRACTOR" >&2
    exit 1
fi

pot=po/featherplay.pot

# 1. Rust: the header, then one entry per (context, id), with every place it's used.
mapfile -t rust < <({ find src -name '*.rs'; find ../../crates/*/src -name '*.rs'; } | sort)
perl -e '
    my $str = qr/"((?:[^"\\]|\\.)*)"/s;
    my (%refs, @order, %plural);
    for my $file (@ARGV) {
        open my $fh, "<:raw", $file or die "$file: $!";
        local $/;
        my $src = <$fh>;
        close $fh;
        # Comment lines (doc comments show the macros) are blanked, keeping the line count.
        $src =~ s{^[ \t]*//[^\n]*}{}mg;
        while ($src =~ /(?<![A-Za-z0-9_])trn?!\(\s*$str\s*(?:=>\s*$str\s*)?(?:\|\s*$str)?/g) {
            my ($ctx, $id) = defined $2 ? ($1, $2) : ("", $1);
            my $line = 1 + (substr($src, 0, $-[0]) =~ tr/\n//);
            my $key = "$ctx\x{1}$id";
            push @order, $key unless $refs{$key};
            push @{ $refs{$key} }, "$file:$line";
            $plural{$key} = $3 if defined $3;
        }
    }
    print "msgid \"\"\nmsgstr \"\"\n";
    print "\"Project-Id-Version: featherplay\\n\"\n";
    print "\"MIME-Version: 1.0\\n\"\n";
    print "\"Content-Type: text/plain; charset=UTF-8\\n\"\n";
    print "\"Content-Transfer-Encoding: 8bit\\n\"\n";
    print "\"Language: \\n\"\n";
    print "\"Plural-Forms: nplurals=2; plural=(n != 1);\\n\"\n";
    for my $key (@order) {
        my ($ctx, $id) = split /\x{1}/, $key, 2;
        print "\n#: ", join(" ", @{ $refs{$key} }), "\n";
        print "msgctxt \"$ctx\"\n" if $ctx ne "";
        print "msgid \"$id\"\n";
        if (defined $plural{$key}) {
            print "msgid_plural \"$plural{$key}\"\nmsgstr[0] \"\"\nmsgstr[1] \"\"\n";
        } else {
            print "msgstr \"\"\n";
        }
    }
' "${rust[@]}" > "$pot"

# 2. Slint, joined into the same template.
mapfile -t slint < <(find ui -name '*.slint' | sort)
"$extractor" --no-default-translation-context -j -o "$pot" "${slint[@]}"

# 3. The languages.
for po in po/*/LC_MESSAGES/featherplay.po; do
    case "$po" in po/qps-ploc/*) continue ;; esac
    if command -v msgmerge >/dev/null; then
        msgmerge --quiet --update --backup=none "$po" "$pot"
    else
        echo "update.sh: msgmerge (GNU gettext) not found; update $po from the template in a translation tool." >&2
    fi
done

# 4. The test language: each msgstr is its msgid (or plural) in brackets.
mkdir -p po/qps-ploc/LC_MESSAGES
perl -e '
    local $/ = "";
    open my $fh, "<:raw", $ARGV[0] or die "$ARGV[0]: $!";
    my @blocks = <$fh>;
    close $fh;
    shift @blocks; # the template header
    print "msgid \"\"\nmsgstr \"\"\n";
    print "\"Project-Id-Version: featherplay\\n\"\n";
    print "\"MIME-Version: 1.0\\n\"\n";
    print "\"Content-Type: text/plain; charset=UTF-8\\n\"\n";
    print "\"Content-Transfer-Encoding: 8bit\\n\"\n";
    print "\"Language: qps-ploc\\n\"\n";
    print "\"Plural-Forms: nplurals=2; plural=(n != 1);\\n\"\n";
    print "\"X-Language-Name: Test language [qps-ploc]\\n\"\n";
    for my $block (@blocks) {
        $block =~ s/\r//g;
        my (%field, $name, @keep);
        for my $line (split /\n/, $block) {
            if ($line =~ /^(msg\S+) "(.*)"$/) {
                $name = $1;
                $field{$name} = $2;
            } elsif ($line =~ /^"(.*)"$/ && defined $name) {
                $field{$name} .= $1;
            } elsif ($line =~ /^#/) {
                push @keep, $line;
                next;
            }
            push @keep, $line unless ($name // "") =~ /^msgstr/;
        }
        print "\n", join("\n", @keep), "\n";
        if (exists $field{msgid_plural}) {
            print "msgstr[0] \"[$field{msgid}]\"\nmsgstr[1] \"[$field{msgid_plural}]\"\n";
        } else {
            print "msgstr \"[$field{msgid}]\"\n";
        }
    }
' "$pot" > po/qps-ploc/LC_MESSAGES/featherplay.po

echo "update.sh: $(grep -c '^msgid "' "$pot") entries in $pot (with the header)"
