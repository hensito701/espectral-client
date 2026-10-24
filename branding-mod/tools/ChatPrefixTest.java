package es.spectral.menu;

/**
 * Standalone harness for {@link ChatPrefix} — the Minecraft-free sender
 * parser used for chat lines that servers deliver as system messages
 * ({@code <[ADMIN] henMV> asd}). Run:
 *
 * <pre>
 *   javac -d build/chatprefix-test src/main/java/es/spectral/menu/ChatPrefix.java \
 *       tools/ChatPrefixTest.java
 *   java -cp build/chatprefix-test es.spectral.menu.ChatPrefixTest
 * </pre>
 *
 * <p>Exit code is non-zero on any failure, so the harness can gate a build.
 */
public final class ChatPrefixTest {

    private static final String HENMV = "henMV";
    private static final String ADMIN = "Admin";
    private static final String VIP = "Vip";

    /** Tab list snapshot used by most cases (plus a two-letter trap name). */
    private static final String[] ROSTER = { HENMV, ADMIN, VIP, "Jo" };

    private static int checks;
    private static int failures;

    public static void main(String[] args) {
        // Plugin chat formats: angle brackets, prefix tags, arrows, pipes.
        expect("angle brackets with prefix", "<[ADMIN] henMV> asd", HENMV);
        expect("colon separator", "[VIP] henMV: hola", HENMV);
        expect("guillemet separator", "✦ [MVP] henMV » texto", HENMV);
        expect("pipe separator", "[ADMIN] henMV | hola", HENMV);
        expect("bare name before colon", "henMV: hola", HENMV);
        expect("angle brackets around the name", "<henMV> hi", HENMV);
        expect("whisper form", "henMV whispers to you: hi", HENMV);
        expect("name as the very first token", "henMV> hola", HENMV);
        expect("decorative separator before name", "» henMV: hola", HENMV);
        expect("timestamp bracket does not end the prefix", "[12:34] henMV: hola", HENMV);
        expect("two prefix tags", "[VIP] [ADMIN] henMV: hola", HENMV);
        expect("guild-like bracket", "✧ [ESPECTRAL] ✧ henMV » gg", HENMV);

        // Broadcasts: no chat separator at all, so no sender.
        expectNone("welcome broadcast", "¡Bienvenido a ESPECTRAL, henMV!");
        expectNone("name with a period, no separator", "Thanks henMV.");
        expectNone("empty line", "");
        expectNone("null line", null);
        expectNone("no name at all", "<[ADMIN] somebodyelse> asd");

        // Name only after the separator: that is the message body.
        expectNone("name only in the body", "<[ADMIN] stranger> henMV was here");
        expectNone("name only in the body, colon form", "[Owner] stranger: hi henMV");
        expectNone("mention in the body", "<stranger> hey henMV, look");

        // Whole-word only: no partial/substring matches.
        expectNone("substring of a longer word", "<henMVtwo> hi");
        expectNone("substring of an underscore name", "<henMV_bot> hi");
        expectNone("name glued to a prefix word", "<ADMINhenMV> hi");
        expectNone("name glued to digits", "<henMV123> hi");

        // Bracketed rank tags are not senders, even when a player shares the tag.
        expect("rank tag ignored in favour of the real name", "[VIP] henMV: hola", HENMV);
        expectNone("only a bracketed roster name", "[VIP] stranger: hola");

        // Case: exact pass first, case-insensitive pass only as a fallback.
        expect("case-insensitive fallback", "<[ADMIN] HENMV> asd", HENMV);
        expect("case-insensitive prefix", "<[admin] henMV> asd", HENMV);

        // Earliest occurrence wins, not roster order.
        expect("earliest wins", "<Vip> Admin: hi", VIP);
        expect("earliest wins reversed", "<Admin> Vip: hi", ADMIN);

        // Two-letter names only match whole words.
        expect("short name whole word", "<Jo> hi", "Jo");
        expectNone("short name inside a word", "<John> hi");

        // Prose that names a player mid-sentence, in neither sender position.
        expectNone("welcome line with a separator claim", "¡Bienvenido henMV! Recuerda: usa /tienda");
        expectNone("name mid-sentence", "Felicidades por la partida de Admin y henMV hoy");
        // Known ambiguity, accepted: a sentence whose last word before a
        // separator is a tab-list name is textually identical to a chat
        // template, so it resolves. It only costs a head on the first line.
        expect("sentence-shaped like a template", "Gran partida de henMV: gg", HENMV);

        // A name ending exactly at the limit still counts.
        String atLimit = repeat('.', ChatPrefix.PREFIX_LIMIT - HENMV.length()) + HENMV + ": hola";
        expect("name ending exactly at the limit", atLimit, HENMV);

        // Prefix length bound: a name pushed past 48 visible chars is ignored.
        String padded = repeat('.', 50) + "henMV: hola";
        expectNone("name past the prefix limit", padded);

        // Names outside the roster are never invented.
        expectNone("stranger name", "<[ADMIN] Stranger> asd");

        // prefixEnd contract.
        check("prefixEnd stops at the first depth-0 separator", 7, ChatPrefix.prefixEnd("<[AD] h>x"));
        check("prefixEnd ignores bracket separators", 7, ChatPrefix.prefixEnd("[a:b] x: y"));
        check("prefixEnd reports no separator", -1, ChatPrefix.prefixEnd("just words here"));
        check("prefixEnd on null", -1, ChatPrefix.prefixEnd(null));

        report();
    }

    private static void expect(String label, String text, String expectedName) {
        String actual = resolve(text);
        checks++;
        if (expectedName.equals(actual)) {
            return;
        }
        failures++;
        System.out.println("FAIL " + label + ": expected '" + expectedName + "', got '"
                + actual + "' for " + quote(text));
    }

    private static void expectNone(String label, String text) {
        String actual = resolve(text);
        checks++;
        if (actual == null) {
            return;
        }
        failures++;
        System.out.println("FAIL " + label + ": expected no sender, got '" + actual
                + "' for " + quote(text));
    }

    private static void check(String label, int expected, int actual) {
        checks++;
        if (expected == actual) {
            return;
        }
        failures++;
        System.out.println("FAIL " + label + ": expected " + expected + ", got " + actual);
    }

    private static String resolve(String text) {
        int index = ChatPrefix.findSender(text, ROSTER);
        return index < 0 ? null : ROSTER[index];
    }

    private static String repeat(char c, int count) {
        StringBuilder builder = new StringBuilder(count);
        for (int i = 0; i < count; i++) builder.append(c);
        return builder.toString();
    }

    private static String quote(String text) {
        return text == null ? "null" : '"' + text + '"';
    }

    private static void report() {
        if (failures == 0) {
            System.out.println("OK: " + checks + " chat prefix checks passed");
            return;
        }
        System.out.println("FAILED: " + failures + " of " + checks + " checks");
        System.exit(1);
    }
}
