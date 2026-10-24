package es.spectral.menu;

/**
 * Minecraft-free sender parser for chat lines that arrive as <em>system</em>
 * messages (Paper servers with a chat plugin, which is how the Espectral
 * servers deliver player chat: {@code [System] [CHAT] <[ADMIN] henMV> asd}).
 *
 * <p>The parser only ever considers names that the caller supplies — in the
 * mod that is an exact snapshot of the current tab list — so it can never
 * invent a sender out of free text. The searched region is the line's
 * <em>sender prefix</em>: everything before the first chat separator
 * ({@code >}, {@code :}, {@code »}, {@code |}) at bracket depth 0, capped at
 * {@link #PREFIX_LIMIT} characters.
 *
 * <p>Inside that prefix only the two positions every chat template uses are
 * tested: the <em>first</em> word token of the line and the <em>last</em> word
 * token before the separator. The first covers vanilla's
 * {@code henMV whispers to you:} shape; the last covers {@code <[ADMIN] henMV>},
 * {@code [VIP] henMV:} and {@code ✦ [MVP] henMV »}. Prose that merely names a
 * player somewhere in the middle of a sentence is ignored.
 *
 * <p>A candidate must also appear <em>whole-word</em> and outside
 * {@code [...]}: brackets are rank/tag slots ({@code [ADMIN]}, {@code [MVP]},
 * {@code [12:34]}), never the sender, so a player literally named {@code Vip}
 * never matches {@code [VIP]}. The earliest of the two positions wins,
 * case-sensitive pass first with a case-insensitive pass only as a fallback.
 *
 * <p>Two deliberate exclusions:
 *
 * <ul>
 *   <li>Lines with no separator at all are rejected. A pure broadcast such as
 *       {@code ¡Bienvenido a ESPECTRAL, henMV!} names a player but has no
 *       sender prefix, so it must not get a head.</li>
 *   <li>Only the prefix is searched, so a name that only appears in the
 *       message body ({@code <[ADMIN] stranger> henMV was here}) never
 *       resolves.</li>
 * </ul>
 *
 * <p>The scans allocate nothing (no substrings, no regex, no arrays) and are
 * bounded by {@link #PREFIX_LIMIT}, so they are cheap enough to run on chat
 * intake. Kept free of Minecraft types on purpose: {@code tools/ChatPrefixTest}
 * exercises it with plain strings.
 */
public final class ChatPrefix {

    /** Characters of the line that are searched for a sender name. */
    public static final int PREFIX_LIMIT = 48;

    /** Characters that terminate the sender prefix in plugin chat formats. */
    private static final char[] SEPARATORS = { '>', ':', '\u00BB', '|' };

    private ChatPrefix() {}

    /**
     * @param text  the line's plain text ({@code Component#getString()})
     * @param names exact candidates (null or empty entries are skipped)
     * @return index of the matching candidate in {@code names}, or -1
     */
    public static int findSender(String text, String[] names) {
        if (text == null || names == null || names.length == 0) return -1;
        int end = prefixEnd(text);
        if (end <= 0) return -1;
        int limit = Math.min(end, PREFIX_LIMIT);
        int first = firstTokenStart(text, limit);
        if (first < 0) return -1;
        int last = lastTokenStart(text, limit);
        int exact = matchTokens(text, names, limit, first, last, true);
        if (exact >= 0) return exact;
        return matchTokens(text, names, limit, first, last, false);
    }

    /**
     * Index of the first chat separator that is outside a {@code [...]} group
     * and after the first word character, or -1 when the line has no such
     * separator (a pure broadcast, or a line whose separators all sit inside
     * brackets).
     */
    public static int prefixEnd(String text) {
        if (text == null) return -1;
        boolean wordSeen = false;
        int depth = 0;
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (c == '[') {
                depth++;
                continue;
            }
            if (c == ']') {
                if (depth > 0) depth--;
                continue;
            }
            if (!wordSeen) {
                if (isWordChar(c)) wordSeen = true;
                continue;
            }
            if (depth == 0 && isSeparator(c)) return i;
        }
        return -1;
    }

    /** Start of the first depth-0 word run, or -1. */
    private static int firstTokenStart(String text, int limit) {
        int depth = 0;
        boolean previousWasWord = false;
        for (int i = 0; i < limit; i++) {
            char c = text.charAt(i);
            if (c == '[') {
                depth++;
                previousWasWord = false;
                continue;
            }
            if (c == ']') {
                if (depth > 0) depth--;
                previousWasWord = false;
                continue;
            }
            boolean word = isWordChar(c);
            if (depth == 0 && word && !previousWasWord) return i;
            previousWasWord = word;
        }
        return -1;
    }

    /** Start of the last depth-0 word run, or -1. */
    private static int lastTokenStart(String text, int limit) {
        int depth = 0;
        boolean previousWasWord = false;
        int last = -1;
        for (int i = 0; i < limit; i++) {
            char c = text.charAt(i);
            if (c == '[') {
                depth++;
                previousWasWord = false;
                continue;
            }
            if (c == ']') {
                if (depth > 0) depth--;
                previousWasWord = false;
                continue;
            }
            boolean word = isWordChar(c);
            if (depth == 0 && word && !previousWasWord) last = i;
            previousWasWord = word;
        }
        return last;
    }

    /**
     * Candidate matching at the first token, then at the last token when that
     * is a different position — i.e. in increasing start order, so the
     * earliest sender-position occurrence wins.
     */
    private static int matchTokens(String text, String[] names, int limit, int first, int last,
            boolean caseSensitive) {
        int found = matchAt(text, names, first, limit, caseSensitive);
        if (found >= 0) return found;
        if (last <= first) return -1;
        return matchAt(text, names, last, limit, caseSensitive);
    }

    /** Candidate equal to the whole word starting at {@code start}, or -1. */
    private static int matchAt(String text, String[] names, int start, int limit,
            boolean caseSensitive) {
        for (int n = 0; n < names.length; n++) {
            String name = names[n];
            if (name == null || name.isEmpty()) continue;
            int after = start + name.length();
            if (after > limit || after > text.length()) continue;
            if (after < text.length() && isWordChar(text.charAt(after))) continue;
            if (!matchesAt(text, start, name, caseSensitive)) continue;
            return n;
        }
        return -1;
    }

    private static boolean matchesAt(String text, int start, String name, boolean caseSensitive) {
        if (caseSensitive) {
            for (int i = 0; i < name.length(); i++) {
                if (text.charAt(start + i) != name.charAt(i)) return false;
            }
            return true;
        }
        return text.regionMatches(true, start, name, 0, name.length());
    }

    /** Letters, digits and underscore — the characters of a Minecraft name. */
    private static boolean isWordChar(char c) {
        return Character.isLetterOrDigit(c) || c == '_';
    }

    private static boolean isSeparator(char c) {
        for (char separator : SEPARATORS) {
            if (c == separator) return true;
        }
        return false;
    }
}
