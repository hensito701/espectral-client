package es.spectral.menu;

import static org.junit.Assert.assertEquals;

import org.junit.Test;

/** Sender detection for chat heads (rules shared with the Fabric mod). */
public class ChatPrefixTest {

    private static final String[] ROSTER = {"henMV", "Admin", "Vip", "Jo"};

    private static String sender(String text) {
        int index = ChatPrefix.findSender(text, ROSTER);
        return index < 0 ? null : ROSTER[index];
    }

    @Test
    public void pluginChatFormatsResolve() {
        assertEquals("henMV", sender("<[ADMIN] henMV> asd"));
        assertEquals("henMV", sender("[VIP] henMV: hola"));
        assertEquals("henMV", sender("[ADMIN] henMV | hola"));
        assertEquals("henMV", sender("<henMV> hi"));
        assertEquals("henMV", sender("[12:34] henMV: hola"));
    }

    @Test
    public void broadcastsBodiesAndPartialNamesDoNot() {
        assertEquals(null, sender("¡Bienvenido a ESPECTRAL, henMV!"));
        assertEquals(null, sender("<stranger> hey henMV, look"));
        assertEquals(null, sender("<henMVtwo> hi"));
        assertEquals(null, sender(""));
        assertEquals(null, sender(null));
    }
}
