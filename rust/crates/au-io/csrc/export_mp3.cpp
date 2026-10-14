// SPDX-License-Identifier: GPL-3.0-or-later
// LAME/libid3tag shim for the Rust MP3 exporter.

#include <lame/lame.h>
#include <id3tag.h>

#include <algorithm>
#include <array>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <exception>
#include <memory>
#include <string>
#include <vector>

struct AuMp3Options {
    int mode;
    int quality_set;
    int quality_vbr;
    int bitrate_abr;
    int bitrate_cbr;
    int channel_mode;
};

struct AuMp3Tag {
    const char *name;
    const char *value;
};

namespace {

constexpr int kSamplesPerChunk = 220500;
constexpr int kOutputBufferSize =
    kSamplesPerChunk * (320 / 8) / 8 +
    4 * 1152 * (320 / 8) / 8 + 512;
constexpr std::size_t kInfoTagBufferSize = 2880;

void set_error(char *buffer, std::size_t capacity, const char *message)
{
    if (buffer == nullptr || capacity == 0)
        return;

    const char *source = message == nullptr ? "MP3 export failed" : message;
    std::snprintf(buffer, capacity, "%s", source);
}

bool is_ascii(const char *text)
{
    if (text == nullptr)
        return true;

    for (const unsigned char *p =
             reinterpret_cast<const unsigned char *>(text);
         *p != 0; ++p) {
        if (*p > 0x7f)
            return false;
    }
    return true;
}

struct Id3TagDeleter {
    void operator()(id3_tag *tag) const
    {
        if (tag != nullptr)
            id3_tag_delete(tag);
    }
};

using Id3TagHolder = std::unique_ptr<id3_tag, Id3TagDeleter>;

struct Ucs4Deleter {
    void operator()(id3_ucs4_t *text) const
    {
        std::free(text);
    }
};

using Ucs4Holder = std::unique_ptr<id3_ucs4_t, Ucs4Deleter>;

struct LameHolder {
    lame_t value{nullptr};

    ~LameHolder()
    {
        if (value != nullptr)
            lame_close(value);
    }
};

struct FileCloser {
    void operator()(FILE *file) const
    {
        if (file != nullptr)
            std::fclose(file);
    }
};

using FileHolder = std::unique_ptr<FILE, FileCloser>;

bool add_frame(id3_tag *tag, const char *name, const char *value,
               const char *frame_name)
{
    id3_frame *frame = id3_frame_new(frame_name);
    if (frame == nullptr)
        return false;

    if (!is_ascii(name) || !is_ascii(value))
        id3_field_settextencoding(
            id3_frame_field(frame, 0), ID3_FIELD_TEXTENCODING_UTF_16);
    else
        id3_field_settextencoding(
            id3_frame_field(frame, 0), ID3_FIELD_TEXTENCODING_ISO_8859_1);

    Ucs4Holder text{id3_utf8_ucs4duplicate(
        reinterpret_cast<const id3_utf8_t *>(value))};
    if (!text)
        return false;

    if (std::strcmp(frame_name, ID3_FRAME_COMMENT) == 0) {
        // Keep Audacity's extra comment frame with a cleared language field.
        id3_frame *frame2 = id3_frame_new(frame_name);
        if (frame2 == nullptr)
            return false;

        id3_field_setfullstring(id3_frame_field(frame2, 3), text.get());
        id3_field *language = id3_frame_field(frame2, 1);
        std::memset(language->immediate.value, 0,
                    sizeof(language->immediate.value));
        id3_tag_attachframe(tag, frame2);

        // The second comment frame retains libid3tag's default "XXX" language.
        id3_field_setfullstring(id3_frame_field(frame, 3), text.get());
    }
    else if (std::strcmp(frame_name, "TXXX") == 0) {
        id3_field_setstring(id3_frame_field(frame, 2), text.get());

        Ucs4Holder description{id3_utf8_ucs4duplicate(
            reinterpret_cast<const id3_utf8_t *>(name))};
        if (!description)
            return false;
        id3_field_setstring(id3_frame_field(frame, 1), description.get());
    }
    else {
        id3_ucs4_t *address = text.get();
        id3_field_setstrings(id3_frame_field(frame, 1), 1, &address);
    }

    id3_tag_attachframe(tag, frame);
    return true;
}

bool build_id3_tag(const AuMp3Tag *tags, std::size_t tag_count,
                   std::vector<unsigned char> &rendered)
{
    Id3TagHolder tag{id3_tag_new()};
    if (!tag)
        return false;

    for (std::size_t i = 0; i < tag_count; ++i) {
        if (tags[i].name == nullptr || tags[i].value == nullptr)
            return false;

        const char *name = tags[i].name;
        const char *value = tags[i].value;
        const char *frame_name = "TXXX";

        if (strcasecmp(name, "TITLE") == 0)
            frame_name = ID3_FRAME_TITLE;
        else if (strcasecmp(name, "ARTIST") == 0)
            frame_name = ID3_FRAME_ARTIST;
        else if (strcasecmp(name, "ALBUM") == 0)
            frame_name = ID3_FRAME_ALBUM;
        else if (strcasecmp(name, "YEAR") == 0) {
            // Audacity writes the legacy TYER frame as well as the newer year frame.
            if (!add_frame(tag.get(), name, value, "TYER"))
                return false;
            frame_name = ID3_FRAME_YEAR;
        }
        else if (strcasecmp(name, "GENRE") == 0)
            frame_name = ID3_FRAME_GENRE;
        else if (strcasecmp(name, "COMMENTS") == 0)
            frame_name = ID3_FRAME_COMMENT;
        else if (strcasecmp(name, "TRACKNUMBER") == 0)
            frame_name = ID3_FRAME_TRACK;

        if (!add_frame(tag.get(), name, value, frame_name))
            return false;
    }

    tag->options &= (~ID3_TAG_OPTION_COMPRESSION);
#ifdef ID3_TAG_HAS_TAG_OPTION_ID3V2_3
    tag->options |= ID3_TAG_OPTION_ID3V2_3;
#endif

    const unsigned long length = id3_tag_render(tag.get(), nullptr);
    if (length == 0)
        return true;

    rendered.resize(length);
    const unsigned long written =
        id3_tag_render(tag.get(), reinterpret_cast<id3_byte_t *>(rendered.data()));
    if (written == 0)
        return false;

    rendered.resize(written);
    return true;
}

int fail(char *error_message, std::size_t error_message_capacity, int code,
         const char *message)
{
    set_error(error_message, error_message_capacity, message);
    return code;
}

} // namespace

extern "C" int au_export_mp3(
    const char *path,
    std::uint32_t sample_rate,
    std::uint32_t channels,
    const float *interleaved,
    std::uint64_t frames,
    const AuMp3Options *options,
    const AuMp3Tag *tags,
    std::size_t tag_count,
    char *error_message,
    std::size_t error_message_capacity)
{
    if (error_message != nullptr && error_message_capacity > 0)
        error_message[0] = '\0';

    if (path == nullptr || options == nullptr)
        return fail(error_message, error_message_capacity, -1,
                    "Invalid MP3 export arguments");
    if (channels == 0 || channels > 2)
        return fail(error_message, error_message_capacity, -2,
                    "MP3 export requires one or two channels");
    if (frames > 0 && interleaved == nullptr)
        return fail(error_message, error_message_capacity, -3,
                    "MP3 input sample buffer is null");
    if (tag_count > 0 && tags == nullptr)
        return fail(error_message, error_message_capacity, -4,
                    "MP3 tag array is null");

    try {
        std::vector<unsigned char> id3_bytes;
        if (!build_id3_tag(tags, tag_count, id3_bytes))
            return fail(error_message, error_message_capacity, -5,
                        "Could not build ID3 metadata");

        LameHolder lame{lame_init()};
        if (lame.value == nullptr)
            return fail(error_message, error_message_capacity, -6,
                        "Could not initialize LAME");

        // Keep the encoder configuration calls in Audacity's source order.
        lame_set_error_protection(lame.value, false);
        lame_set_num_channels(lame.value, static_cast<int>(channels));
        lame_set_in_samplerate(lame.value, static_cast<int>(sample_rate));
        lame_set_out_samplerate(lame.value, static_cast<int>(sample_rate));
        lame_set_disable_reservoir(lame.value, false);
        // Add the VbrTag for all types: Xing for ABR/VBR and Lame Info for CBR.
        lame_set_bWriteVbrTag(lame.value, true);

        switch (options->mode) {
        case 0: {
            int preset;
            if (options->quality_set == 0)
                preset = INSANE;
            else if (options->quality_set == 1)
                preset = EXTREME_FAST;
            else if (options->quality_set == 2)
                preset = STANDARD_FAST;
            else
                preset = 1007; // Not defined until LAME 3.96.
            lame_set_preset(lame.value, preset);
        } break;

        case 1:
            lame_set_VBR(lame.value, vbr_mtrh);
            lame_set_VBR_q(lame.value, options->quality_vbr);
            break;

        case 2:
            lame_set_preset(lame.value, options->bitrate_abr);
            break;

        default:
            lame_set_VBR(lame.value, vbr_off);
            lame_set_brate(lame.value, options->bitrate_cbr);
            break;
        }

        MPEG_mode mode;
        if (channels == 1)
            mode = MONO;
        else
            mode = options->channel_mode == 1 ? STEREO : JOINT_STEREO;
        lame_set_mode(lame.value, mode);

        const int init_result = lame_init_params(lame.value);
        if (init_result < 0)
            return fail(error_message, error_message_capacity, -7,
                        "Unable to initialize MP3 stream");

        FileHolder file{std::fopen(path, "w+b")};
        if (!file)
            return fail(error_message, error_message_capacity, -8,
                        "Unable to open target file for writing");

        if (!id3_bytes.empty() &&
            std::fwrite(id3_bytes.data(), 1, id3_bytes.size(), file.get()) !=
                id3_bytes.size())
            return fail(error_message, error_message_capacity, -9,
                        "Could not write ID3 metadata");

        const long info_tag_position = std::ftell(file.get());
        if (info_tag_position < 0)
            return fail(error_message, error_message_capacity, -10,
                        "Could not seek in MP3 output file");

        std::vector<unsigned char> output(kOutputBufferSize);
        std::uint64_t position = 0;
        while (position < frames) {
            const int sample_count = static_cast<int>(
                std::min<std::uint64_t>(kSamplesPerChunk, frames - position));
            const float *input =
                interleaved + position * static_cast<std::uint64_t>(channels);

            int encoded;
            if (channels > 1) {
                encoded = lame_encode_buffer_interleaved_ieee_float(
                    lame.value, input, sample_count, output.data(),
                    static_cast<int>(output.size()));
            }
            else if (sample_count == kSamplesPerChunk) {
                encoded = lame_encode_buffer_ieee_float(
                    lame.value, input, input, kSamplesPerChunk, output.data(),
                    static_cast<int>(output.size()));
            }
            else {
                encoded = lame_encode_buffer_ieee_float(
                    lame.value, input, input, sample_count, output.data(),
                    static_cast<int>(output.size()));
            }

            if (encoded < 0)
                return fail(error_message, error_message_capacity, -11,
                            "LAME returned an error while encoding");
            if (encoded > 0 &&
                std::fwrite(output.data(), 1, static_cast<std::size_t>(encoded),
                            file.get()) != static_cast<std::size_t>(encoded))
                return fail(error_message, error_message_capacity, -12,
                            "Could not write encoded MP3 data");

            position += static_cast<std::uint64_t>(sample_count);
        }

        const int flushed = lame_encode_flush(
            lame.value, output.data(), static_cast<int>(output.size()));
        if (flushed < 0)
            return fail(error_message, error_message_capacity, -13,
                        "Could not finish MP3 encoding");
        if (flushed > 0 &&
            std::fwrite(output.data(), 1, static_cast<std::size_t>(flushed),
                        file.get()) != static_cast<std::size_t>(flushed))
            return fail(error_message, error_message_capacity, -14,
                        "Could not write final MP3 data");

        std::array<unsigned char, kInfoTagBufferSize> info_tag_buffer{};
        const std::size_t info_tag_length = lame_get_lametag_frame(
            lame.value, info_tag_buffer.data(), info_tag_buffer.size());

        // Audacity writes the LAME/Xing information tag at the original
        // position of the first encoded frame.
        if (info_tag_length > 0) {
            if (std::fseek(file.get(), info_tag_position, SEEK_SET) != 0)
                return fail(error_message, error_message_capacity, -15,
                            "Could not seek to the MP3 information tag");
            if (std::fwrite(info_tag_buffer.data(), 1, info_tag_length,
                            file.get()) != info_tag_length)
                return fail(error_message, error_message_capacity, -16,
                            "Could not write the MP3 information tag");
        }
        else {
            lame_mp3_tags_fid(lame.value, file.get());
        }

        if (std::fseek(file.get(), 0, SEEK_END) != 0 ||
            std::fflush(file.get()) != 0)
            return fail(error_message, error_message_capacity, -17,
                        "Could not finalize MP3 output file");

        return 0;
    }
    catch (const std::exception &exception) {
        return fail(error_message, error_message_capacity, -18,
                    exception.what());
    }
    catch (...) {
        return fail(error_message, error_message_capacity, -19,
                    "Unknown MP3 export failure");
    }
}
