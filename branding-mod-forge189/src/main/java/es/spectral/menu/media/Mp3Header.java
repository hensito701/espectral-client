package es.spectral.menu.media;

import java.io.BufferedInputStream;
import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.io.RandomAccessFile;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;

/**
 * MP3 container helpers: ID3v2/ID3v1 tags and the frame-walked duration.
 *
 * <p>The duration walk reads the file sequentially through a small buffer and
 * never loads it: memory stays constant whatever the file size. VBR files get
 * an exact length this way (a bitrate estimate would be wrong for them), and
 * the walk is run by {@link MediaLibrary} on its own daemon thread, never on
 * the render thread.
 */
public final class Mp3Header {

    /** Guard against a corrupt header chain claiming a huge duration. */
    private static final int MAX_FRAMES = 400_000;

    private static final int[] SAMPLE_RATES_V1 = {44100, 48000, 32000, 0};
    private static final int[] SAMPLE_RATES_V2 = {22050, 24000, 16000, 0};
    private static final int[] SAMPLE_RATES_V25 = {11025, 12000, 8000, 0};
    private static final int[] BITRATES_V1_L3 = {0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0};
    private static final int[] BITRATES_V2_L3 = {0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0};

    private Mp3Header() {}

    /** True for the extensions this player can open. */
    public static boolean isPlayableName(String name) {
        String lower = name.toLowerCase(Locale.ROOT);
        return lower.endsWith(".mp3");
    }

    /** Title/artist tags of one file; empty strings when the file carries none. */
    public static String[] readTags(Path path) {
        String title = "";
        String artist = "";
        try (InputStream in = Files.newInputStream(path)) {
            byte[] head = readFully(in, 10);
            if (head != null && head.length == 10 && head[0] == 'I' && head[1] == 'D'
                    && head[2] == '3') {
                int size = ((head[6] & 0x7F) << 21) | ((head[7] & 0x7F) << 14)
                        | ((head[8] & 0x7F) << 7) | (head[9] & 0x7F);
                byte[] tag = readFully(in, Math.min(size, 1 << 20));
                if (tag != null) {
                    title = id3v2Frame(tag, "TIT2");
                    artist = id3v2Frame(tag, "TPE1");
                }
            }
        } catch (Exception ignored) {
            // Tags are cosmetic: a broken tag must never hide a playable file.
        }
        if (title.isEmpty() || artist.isEmpty()) {
            String[] v1 = readId3v1(path);
            if (title.isEmpty()) {
                title = v1[0];
            }
            if (artist.isEmpty()) {
                artist = v1[1];
            }
        }
        if (title.isEmpty()) {
            String file = path.getFileName() == null ? "" : path.getFileName().toString();
            int dot = file.lastIndexOf('.');
            title = dot > 0 ? file.substring(0, dot) : file;
        }
        return new String[] {title, artist};
    }

    private static String id3v2Frame(byte[] tag, String id) {
        int offset = 0;
        while (offset + 10 <= tag.length) {
            String frameId = new String(tag, offset, 4, StandardCharsets.ISO_8859_1);
            if (frameId.charAt(0) == 0) {
                break;
            }
            int frameSize = ((tag[offset + 4] & 0xFF) << 24) | ((tag[offset + 5] & 0xFF) << 16)
                    | ((tag[offset + 6] & 0xFF) << 8) | (tag[offset + 7] & 0xFF);
            if (frameSize <= 0 || offset + 10 + frameSize > tag.length) {
                break;
            }
            if (id.equals(frameId)) {
                int start = offset + 10;
                if ((tag[start] & 0xFF) == 1 || (tag[start] & 0xFF) == 2) {
                    // UTF-16 with BOM: decode as UTF-16LE/BE via the BOM bytes.
                    return new String(tag, start + 1, frameSize - 1,
                            (tag[start] & 0xFF) == 1 ? StandardCharsets.UTF_16 : StandardCharsets.UTF_16BE)
                            .replace("\u0000", "").trim();
                }
                return new String(tag, start + 1, frameSize - 1, StandardCharsets.ISO_8859_1)
                        .replace("\u0000", "").trim();
            }
            offset += 10 + frameSize;
        }
        return "";
    }

    private static String[] readId3v1(Path path) {
        String[] out = {"", ""};
        try (RandomAccessFile raf = new RandomAccessFile(path.toFile(), "r")) {
            if (raf.length() < 128) {
                return out;
            }
            raf.seek(raf.length() - 128);
            byte[] tag = new byte[128];
            raf.readFully(tag);
            if (tag[0] == 'T' && tag[1] == 'A' && tag[2] == 'G') {
                out[0] = new String(tag, 3, 30, StandardCharsets.ISO_8859_1)
                        .replace("\u0000", "").trim();
                out[1] = new String(tag, 33, 30, StandardCharsets.ISO_8859_1)
                        .replace("\u0000", "").trim();
            }
        } catch (Exception ignored) {
            // Same rule as the v2 tags: never fail the scan because of metadata.
        }
        return out;
    }

    /**
     * Skips a leading ID3v2 tag and returns {@code in}, now at the audio data.
     * The MPEG decoder knows no tags and syncs on the first 0xFFF it meets, so
     * cover art inside the tag decodes as a bogus frame and the file fails to
     * open. {@code in} must support mark/reset.
     */
    public static InputStream skipId3v2(InputStream in) throws IOException {
        in.mark(10);
        byte[] head = new byte[10];
        int got = 0;
        while (got < head.length) {
            int n = in.read(head, got, head.length - got);
            if (n < 0) {
                break;
            }
            got += n;
        }
        if (got < head.length || head[0] != 'I' || head[1] != 'D' || head[2] != '3') {
            in.reset();
            return in;
        }
        long left = (((head[6] & 0x7F) << 21) | ((head[7] & 0x7F) << 14)
                | ((head[8] & 0x7F) << 7) | (head[9] & 0x7F))
                + ((head[5] & 0x10) != 0 ? 10 : 0); // footer flag
        while (left > 0) {
            long n = in.skip(left);
            if (n <= 0) {
                if (in.read() < 0) {
                    throw new EOFException();
                }
                n = 1;
            }
            left -= n;
        }
        return in;
    }

    /**
     * Exact length by walking MPEG frame headers; {@code -1} when the file does
     * not look like an MPEG audio stream.
     */
    public static long durationMs(Path path) {
        try (InputStream in = Files.newInputStream(path)) {
            BufferedInputStream buf = new BufferedInputStream(in, 64 * 1024);
            Mp3Header.skipId3v2(buf);
            int b0 = buf.read();
            int b1 = buf.read();
            int b2 = buf.read();
            long totalFrames = 0L;
            double seconds = 0.0;
            int header = (b0 << 16) | (b1 << 8) | b2;
            int frames = 0;
            while (frames < MAX_FRAMES) {
                int h = header;
                int versionBits = (h >>> 19) & 0x3;
                int layerBits = (h >>> 17) & 0x3;
                int bitrateIndex = (h >>> 12) & 0xF;
                int rateIndex = (h >>> 10) & 0x3;
                int padding = (h >>> 9) & 0x1;
                int sampleRate = sampleRate(versionBits, rateIndex);
                int frameSamples = versionBits == 3 ? 1152 : 576;
                int bitrate = bitrateKbps(versionBits, layerBits, bitrateIndex);
                int frameLength = frameLength(versionBits, layerBits, bitrate, sampleRate, padding);
                if (sampleRate <= 0 || bitrate <= 0 || frameLength <= 0) {
                    // Resync: try the next byte as a header start.
                    int next = buf.read();
                    if (next < 0) {
                        break;
                    }
                    header = ((header << 8) | next) & 0xFFFFFF;
                    continue;
                }
                totalFrames++;
                seconds += frameSamples / (double) sampleRate;
                long toSkip = frameLength - 3L;
                while (toSkip > 0) {
                    long n = buf.skip(toSkip);
                    if (n <= 0) {
                        break;
                    }
                    toSkip -= n;
                }
                int h0 = buf.read();
                int h1 = buf.read();
                int h2 = buf.read();
                if (h2 < 0) {
                    break;
                }
                header = (h0 << 16) | (h1 << 8) | h2;
                frames++;
            }
            if (totalFrames == 0) {
                return -1L;
            }
            return Math.round(seconds * 1000.0);
        } catch (Exception e) {
            return -1L;
        }
    }

    private static int sampleRate(int versionBits, int rateIndex) {
        if (rateIndex == 3) {
            return 0;
        }
        switch (versionBits) {
            case 3: return SAMPLE_RATES_V1[rateIndex];
            case 2: return SAMPLE_RATES_V2[rateIndex];
            case 0: return SAMPLE_RATES_V25[rateIndex];
            default: return 0;
        }
    }

    private static int bitrateKbps(int versionBits, int layerBits, int index) {
        if (index == 0 || index == 15 || layerBits != 1) {
            // Only Layer III is supported (what every .mp3 is).
            return 0;
        }
        return versionBits == 3 ? BITRATES_V1_L3[index] : BITRATES_V2_L3[index];
    }

    private static int frameLength(int versionBits, int layerBits, int bitrateKbps,
            int sampleRate, int padding) {
        if (bitrateKbps <= 0 || sampleRate <= 0) {
            return 0;
        }
        int bitrate = bitrateKbps * 1000;
        if (versionBits == 3) {
            return 144 * bitrate / sampleRate + padding;
        }
        return 72 * bitrate / sampleRate + padding;
    }

    private static byte[] readFully(InputStream in, int len) throws IOException {
        byte[] out = new byte[len];
        int read = 0;
        while (read < len) {
            int n = in.read(out, read, len - read);
            if (n < 0) {
                throw new EOFException();
            }
            read += n;
        }
        return out;
    }
}
