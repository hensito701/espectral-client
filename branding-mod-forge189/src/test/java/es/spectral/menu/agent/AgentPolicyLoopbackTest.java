package es.spectral.menu.agent;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

/**
 * Loopback is a property of the address, not of a name's text: a public
 * server reached as {@code 127.attacker.example} must not get the act tier.
 */
public class AgentPolicyLoopbackTest {

    @Test
    public void dnsNamesStartingWith127AreNotLoopback() {
        assertFalse(AgentPolicy.isLoopback("127.attacker.example"));
        assertFalse(AgentPolicy.isLoopback("127.0.0.1.nip.io"));
        assertFalse(AgentPolicy.isLoopback("127.0.0.256"));
        assertFalse(AgentPolicy.isLoopback("127.1"));
        assertEquals(AgentPolicy.Context.SERVER, AgentPolicy.classify("127.attacker.test:25565", "198.51.100.7"));
        assertEquals(AgentPolicy.Context.SERVER, AgentPolicy.classify("127.attacker.test:25565", null));
    }

    @Test
    public void realLoopbackAddressesStillCount() {
        assertTrue(AgentPolicy.isLoopback("127.0.0.5"));
        assertTrue(AgentPolicy.isLoopback("localhost"));
        assertTrue(AgentPolicy.isLoopback("::1"));
        assertEquals(AgentPolicy.Context.LOCALHOST, AgentPolicy.classify("127.0.0.5", null));
        assertEquals(AgentPolicy.Context.LOCALHOST, AgentPolicy.classify("localhost:25565", "127.0.0.1"));
        // A name that resolves to a public address is judged by the socket.
        assertEquals(AgentPolicy.Context.SERVER, AgentPolicy.classify("localhost", "198.51.100.7"));
    }
}
