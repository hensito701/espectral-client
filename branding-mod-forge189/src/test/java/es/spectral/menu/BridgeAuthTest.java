package es.spectral.menu;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import java.net.URL;
import java.nio.charset.Charset;

import org.junit.Test;

/** Wire vectors shared with the engine (native/engine/src/proof.rs). */
public class BridgeAuthTest {

    private static final Charset UTF8 = Charset.forName("UTF-8");
    private static final String TOKEN = repeat('a', 64);
    private static final String NONCE = "0123456789abcdef0123456789abcdef";
    private static final byte[] BODY = "{\"uuids\":[]}".getBytes(UTF8);

    private static String repeat(char c, int n) {
        StringBuilder sb = new StringBuilder(n);
        for (int i = 0; i < n; i++) sb.append(c);
        return sb.toString();
    }

    @Test
    public void hmacMatchesRfc4231() {
        assertEquals("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
                BridgeAuth.hmacHex("Jefe".getBytes(UTF8), "what do ya want for nothing?".getBytes(UTF8)));
    }

    @Test
    public void sameIdMacAndProofAsTheEngine() throws Exception {
        BridgeAuth auth = new BridgeAuth(TOKEN);
        assertEquals("ffe054fe7ae0cb6d", auth.id());
        String target = BridgeAuth.target(new URL("http://127.0.0.1:4199/api/bridge/visible"));
        assertEquals("/api/bridge/visible", target);
        assertEquals("d72611961d5c0ee6dbab6f30bdc227b0db1ac81228d96a141d16f455a19ea18f",
                auth.requestMac("post", target, NONCE, BODY));
        assertTrue(auth.proofOk(NONCE, BODY,
                "ec7f663724dd302b4bc9713436253e7dcb7158ae36d1ad8657f9b131cb3d5cdf"));
    }

    /** An answer from anything but the launcher (no or wrong proof) is refused. */
    @Test
    public void unprovenAnswersAreRefused() {
        BridgeAuth auth = new BridgeAuth(TOKEN);
        String good = "ec7f663724dd302b4bc9713436253e7dcb7158ae36d1ad8657f9b131cb3d5cdf";
        assertFalse(auth.proofOk(NONCE, BODY, null));
        assertFalse(auth.proofOk(NONCE, "{\"uuids\":[1]}".getBytes(UTF8), good));
        assertFalse(auth.proofOk("ffffffffffffffffffffffffffffffff", BODY, good));
        assertFalse(new BridgeAuth(repeat('b', 64)).proofOk(NONCE, BODY, good));
        assertEquals(32, BridgeAuth.newNonce().length());
    }
}
