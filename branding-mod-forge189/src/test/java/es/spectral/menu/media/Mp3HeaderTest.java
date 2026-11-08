package es.spectral.menu.media;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import java.io.BufferedInputStream;
import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;

import org.junit.Test;

import fr.delthas.javamp3.Sound;

/**
 * MP3s with an ID3v2 tag (cover art, as YouTube downloaders write it) must
 * open: the decoder has no tag support and syncs on the first 0xFFF it finds,
 * which inside a picture decodes as a bogus Layer I frame and throws.
 */
public class Mp3HeaderTest {

    /** MPEG-1 Layer III, 128 kbps, 44.1 kHz, joint stereo, no CRC: 417 bytes per frame. */
    private static final byte[] FRAME_HEADER = {(byte) 0xFF, (byte) 0xFB, (byte) 0x90, (byte) 0x64};
    private static final int FRAME_LEN = 417;

    private static byte[] silentFrames(int n) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        for (int i = 0; i < n; i++) {
            out.write(FRAME_HEADER, 0, FRAME_HEADER.length);
            out.write(new byte[FRAME_LEN - FRAME_HEADER.length], 0, FRAME_LEN - FRAME_HEADER.length);
        }
        return out.toByteArray();
    }

    /** An ID3v2.3 tag whose body holds a false frame sync with an out-of-range Layer I bit allocation. */
    private static byte[] tagWithFalseSync(boolean footer) {
        byte[] body = new byte[300];
        body[40] = (byte) 0xFF;
        body[41] = (byte) 0xFF;
        for (int i = 42; i < body.length; i++) {
            body[i] = (byte) 0xFF;
        }
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.write(new byte[] {'I', 'D', '3', 3, 0, (byte) (footer ? 0x10 : 0),
                0, 0, (byte) (body.length >> 7), (byte) (body.length & 0x7F)}, 0, 10);
        out.write(body, 0, body.length);
        if (footer) {
            out.write(new byte[] {'3', 'D', 'I', 3, 0, 0x10, 0, 0, (byte) (body.length >> 7),
                    (byte) (body.length & 0x7F)}, 0, 10);
        }
        return out.toByteArray();
    }

    private static byte[] concat(byte[] a, byte[] b) {
        byte[] out = new byte[a.length + b.length];
        System.arraycopy(a, 0, out, 0, a.length);
        System.arraycopy(b, 0, out, a.length, b.length);
        return out;
    }

    private static long decodedBytes(byte[] file) throws Exception {
        InputStream in = new BufferedInputStream(new ByteArrayInputStream(file));
        try (Sound sound = new Sound(Mp3Header.skipId3v2(in))) {
            assertEquals(44100f, sound.getAudioFormat().getSampleRate(), 0f);
            byte[] buf = new byte[8192];
            long total = 0;
            int n;
            while ((n = sound.read(buf, 0, buf.length)) >= 0) {
                total += n;
            }
            return total;
        }
    }

    @Test
    public void taggedFileDecodesLikeTheBareStream() throws Exception {
        byte[] audio = silentFrames(6);
        long bare = decodedBytes(audio);
        assertTrue("bare stream decodes", bare > 0);
        assertEquals(bare, decodedBytes(concat(tagWithFalseSync(false), audio)));
        assertEquals(bare, decodedBytes(concat(tagWithFalseSync(true), audio)));
    }

    @Test
    public void untaggedStreamIsLeftAtItsFirstByte() throws Exception {
        InputStream in = new BufferedInputStream(new ByteArrayInputStream(new byte[] {(byte) 0xFF, (byte) 0xFB, 1, 2}));
        Mp3Header.skipId3v2(in);
        assertEquals(0xFF, in.read());
        assertEquals(0xFB, in.read());
    }
}
