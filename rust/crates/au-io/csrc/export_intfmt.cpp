// Audacity-compatible export backend shim.
// SPDX-License-Identifier: GPL-3.0-or-later

#include <sndfile.h>
#include <FLAC/metadata.h>
#include <FLAC/stream_encoder.h>
#include <wavpack/wavpack.h>
#include <id3tag.h>
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>
extern "C" {
struct id3_frame *id3_frame_new(char const *);
id3_length_t id3_latin1_length(id3_latin1_t const *);
void id3_latin1_decode(id3_latin1_t const *, id3_ucs4_t *);
}

namespace {

struct TagView {
    const char *name;
    const char *value;
};

void set_error(char *buffer, size_t length, const std::string &message)
{
    if (!buffer || length == 0)
        return;
    const size_t count = std::min(length - 1, message.size());
    std::memcpy(buffer, message.data(), count);
    buffer[count] = '\0';
}

std::string sndfile_error(SNDFILE *file)
{
    const char *message = file ? sf_strerror(file) : sf_error_number(sf_error(nullptr));
    return message ? message : "libsndfile export failed";
}

const char *tag_value(const TagView *tags, size_t count, const char *key)
{
    for (size_t i = 0; i < count; ++i) {
        if (tags[i].name && tags[i].value &&
            strcasecmp(tags[i].name, key) == 0)
            return tags[i].value;
    }
    return nullptr;
}

void set_string(SNDFILE *file, int id, const char *value)
{
    if (value && *value)
        sf_set_string(file, id, value);
}

void add_sndfile_strings(SNDFILE *file, const TagView *tags, size_t count)
{
    set_string(file, SF_STR_TITLE, tag_value(tags, count, "TITLE"));
    set_string(file, SF_STR_ALBUM, tag_value(tags, count, "ALBUM"));
    set_string(file, SF_STR_ARTIST, tag_value(tags, count, "ARTIST"));
    set_string(file, SF_STR_COMMENT, tag_value(tags, count, "COMMENTS"));
    set_string(file, SF_STR_DATE, tag_value(tags, count, "YEAR"));
    set_string(file, SF_STR_GENRE, tag_value(tags, count, "GENRE"));
    set_string(file, SF_STR_COPYRIGHT, tag_value(tags, count, "COPYRIGHT"));
    set_string(file, SF_STR_SOFTWARE, tag_value(tags, count, "SOFTWARE"));
    set_string(file, SF_STR_TRACKNUMBER, tag_value(tags, count, "TRACKNUMBER"));
}

bool add_id3_chunk(const char *path, int format, const TagView *tags, size_t count)
{
    id3_tag *tag = id3_tag_new();
    if (!tag)
        return false;

    for (size_t i = 0; i < count; ++i) {
        const char *name = tags[i].name ? tags[i].name : "";
        const char *value = tags[i].value ? tags[i].value : "";
        const char *frame_name = "TXXX";
        if (strcasecmp(name, "TITLE") == 0)
            frame_name = ID3_FRAME_TITLE;
        else if (strcasecmp(name, "ARTIST") == 0)
            frame_name = ID3_FRAME_ARTIST;
        else if (strcasecmp(name, "ALBUM") == 0)
            frame_name = ID3_FRAME_ALBUM;
        else if (strcasecmp(name, "YEAR") == 0)
            frame_name = ID3_FRAME_YEAR;
        else if (strcasecmp(name, "GENRE") == 0)
            frame_name = ID3_FRAME_GENRE;
        else if (strcasecmp(name, "COMMENTS") == 0)
            frame_name = ID3_FRAME_COMMENT;
        else if (strcasecmp(name, "TRACKNUMBER") == 0)
            frame_name = ID3_FRAME_TRACK;
        else if (strcasecmp(name, "composer") == 0)
            frame_name = "TCOM";

        id3_frame *frame = id3_frame_new(frame_name);
        if (!frame) {
            id3_tag_delete(tag);
            return false;
        }

        const bool ascii = std::all_of(name, name + std::strlen(name),
            [](unsigned char ch) { return ch < 0x80; }) &&
            std::all_of(value, value + std::strlen(value),
            [](unsigned char ch) { return ch < 0x80; });
        id3_field_settextencoding(id3_frame_field(frame, 0),
            ascii ? ID3_FIELD_TEXTENCODING_ISO_8859_1
                  : ID3_FIELD_TEXTENCODING_UTF_16);

        id3_ucs4_t *ucs4 = id3_utf8_ucs4duplicate(
            reinterpret_cast<const id3_utf8_t *>(value));
        if (!ucs4) {
            id3_frame_delete(frame);
            id3_tag_delete(tag);
            return false;
        }

        if (std::strcmp(frame_name, ID3_FRAME_COMMENT) == 0) {
            id3_field *language = id3_frame_field(frame, 1);
            std::memset(language->immediate.value, 0, sizeof(language->immediate.value));
            id3_field_setfullstring(id3_frame_field(frame, 3), ucs4);
        } else if (std::strcmp(frame_name, "TXXX") == 0) {
            id3_field_setstring(id3_frame_field(frame, 2), ucs4);
            id3_ucs4_t *description = id3_utf8_ucs4duplicate(
                reinterpret_cast<const id3_utf8_t *>(name));
            if (description) {
                id3_field_setstring(id3_frame_field(frame, 1), description);
                std::free(description);
            }
        } else {
            id3_field_setstrings(id3_frame_field(frame, 1), 1, &ucs4);
        }

        std::free(ucs4);
        id3_tag_attachframe(tag, frame);
    }

    tag->options &= ~ID3_TAG_OPTION_COMPRESSION;
#ifdef ID3_TAG_HAS_TAG_OPTION_ID3V2_3
    tag->options |= ID3_TAG_OPTION_ID3V2_3;
#endif

    id3_length_t length = id3_tag_render(tag, nullptr);
    if (length == 0) {
        id3_tag_delete(tag);
        return true;
    }
    if (length & 1)
        ++length;
    std::vector<id3_byte_t> bytes(length, 0);
    id3_tag_render(tag, bytes.data());
    id3_tag_delete(tag);

    FILE *file = std::fopen(path, "r+b");
    if (!file)
        return false;
    if (std::fseek(file, 0, SEEK_END) != 0) {
        std::fclose(file);
        return false;
    }
    const bool wav = (format & SF_FORMAT_TYPEMASK) == SF_FORMAT_WAV;
    if (std::fwrite(wav ? "id3 " : "ID3 ", 1, 4, file) != 4) {
        std::fclose(file);
        return false;
    }
    uint32_t chunk_size = static_cast<uint32_t>(length);
    uint32_t disk_size = chunk_size;
    const bool aiff = (format & SF_FORMAT_TYPEMASK) == SF_FORMAT_AIFF;
    if (!wav) {
        const uint32_t n = disk_size;
        disk_size = ((n & 0xffU) << 24) | ((n & 0xff00U) << 8) |
                    ((n & 0xff0000U) >> 8) | ((n & 0xff000000U) >> 24);
    }
    if (std::fwrite(&disk_size, 1, 4, file) != 4 ||
        std::fwrite(bytes.data(), 1, bytes.size(), file) != bytes.size()) {
        std::fclose(file);
        return false;
    }

    const long end = std::ftell(file);
    if (end < 8 || std::fseek(file, 4, SEEK_SET) != 0) {
        std::fclose(file);
        return false;
    }
    uint32_t container_size = static_cast<uint32_t>(end - 8);
    if (aiff) {
        const uint32_t n = container_size;
        container_size = ((n & 0xffU) << 24) | ((n & 0xff00U) << 8) |
                         ((n & 0xff0000U) >> 8) | ((n & 0xff000000U) >> 24);
    }
    const bool ok = std::fwrite(&container_size, 1, 4, file) == 4 &&
                    std::fflush(file) == 0;
    return std::fclose(file) == 0 && ok;
}

FLAC__StreamMetadata_VorbisComment_Entry entry_from_pair(const char *n, const char *v)
{
    FLAC__StreamMetadata_VorbisComment_Entry e{0, nullptr};
    FLAC__metadata_object_vorbiscomment_entry_from_name_value_pair(&e, n, v);
    return e;
}

FLAC__StreamMetadata *make_flac_metadata(const TagView *tags, size_t count)
{
    FLAC__StreamMetadata *metadata =
        FLAC__metadata_object_new(FLAC__METADATA_TYPE_VORBIS_COMMENT);
    if (!metadata)
        return nullptr;

    for (size_t i = 0; i < count; ++i) {
        const std::string name = tags[i].name ? tags[i].name : "";
        const std::string value = tags[i].value ? tags[i].value : "";
        std::string field = name;
        if (strcasecmp(name.c_str(), "YEAR") == 0)
            field = "DATE";
        if (strcasecmp(name.c_str(), "COMMENTS") == 0) {
            if (!FLAC__metadata_object_vorbiscomment_append_comment(
                    metadata,
                    entry_from_pair("COMMENT", value.c_str()),
                    false) ||
                !FLAC__metadata_object_vorbiscomment_append_comment(
                    metadata,
                    entry_from_pair("DESCRIPTION", value.c_str()),
                    true)) {
                FLAC__metadata_object_delete(metadata);
                return nullptr;
            }
            continue;
        }

        if (!FLAC__metadata_object_vorbiscomment_append_comment(
                metadata,
                entry_from_pair(field.c_str(), value.c_str()),
                true)) {
            FLAC__metadata_object_delete(metadata);
            return nullptr;
        }
    }
    return metadata;
}

// Audacity's FileFormats.cpp sf_subtype_is_integer.
bool sf_subtype_is_integer(unsigned int format)
{
    unsigned int subtype = format & SF_FORMAT_SUBMASK;
    return (subtype == SF_FORMAT_PCM_16 || subtype == SF_FORMAT_PCM_24 ||
            subtype == SF_FORMAT_PCM_32);
}

struct WvWriter {
    FILE *wv = nullptr;
    FILE *wvc = nullptr;
    uint32_t first_block_size = 0;
};

int write_wavpack_block(void *opaque, void *data, int32_t length)
{
    if (!opaque || !data || length == 0)
        return 1;
    auto *writer = static_cast<WvWriter *>(opaque);
    FILE *file = writer->wv;
    if (!file)
        return 0;
    if (std::fwrite(data, 1, static_cast<size_t>(length), file) !=
        static_cast<size_t>(length))
        return 0;
    if (writer->first_block_size == 0)
        writer->first_block_size = static_cast<uint32_t>(length);
    return 1;
}

int write_wavpack_correction(void *opaque, void *data, int32_t length)
{
    if (!opaque || !data || length == 0)
        return 1;
    auto *writer = static_cast<WvWriter *>(opaque);
    if (!writer->wvc)
        return 0;
    return std::fwrite(data, 1, static_cast<size_t>(length), writer->wvc) ==
           static_cast<size_t>(length);
}

} // namespace

extern "C" int aud_export_pcm(
    const char *path, uint32_t sample_rate, uint32_t channels,
    const void *samples, int sample_kind, uint64_t frames,
    const void *opaque_options, const char *const *names,
    const char *const *values, size_t tag_count, char *error,
    size_t error_length)
{
    struct PcmOptionsC {
        int header;
        int encoding;
    };
    const auto *options = static_cast<const PcmOptionsC *>(opaque_options);
    if (!path || !options || channels == 0) {
        set_error(error, error_length, "invalid PCM export arguments");
        return -1;
    }

    SF_INFO info{};
    info.samplerate = static_cast<int>(sample_rate);
    info.channels = static_cast<int>(channels);
    info.format = options->header | options->encoding;
    info.frames = static_cast<sf_count_t>(frames);
    info.sections = 1;
    info.seekable = 0;

    if ((options->encoding == SF_FORMAT_GSM610 && channels != 1) ||
        (info.format == (SF_FORMAT_WAVEX | SF_FORMAT_GSM610))) {
        set_error(error, error_length, "unsupported channel count or PCM format");
        return -2;
    }
    if (!sf_format_check(&info)) {
        info.format = options->header;
        if (!sf_format_check(&info)) {
            set_error(error, error_length, "cannot export audio in this format");
            return -3;
        }
    }

    SNDFILE *file = sf_open(path, SFM_WRITE, &info);
    if (!file) {
        set_error(error, error_length, sndfile_error(nullptr));
        return -4;
    }
    sf_command(file, SFC_SET_CLIPPING, nullptr,
        sf_subtype_is_integer(info.format) ? SF_TRUE : SF_FALSE);

    std::vector<TagView> tags(tag_count);
    for (size_t i = 0; i < tag_count; ++i)
        tags[i] = {names ? names[i] : nullptr, values ? values[i] : nullptr};

    const int type = info.format & SF_FORMAT_TYPEMASK;
    if (type != SF_FORMAT_WAV && type != SF_FORMAT_WAVEX)
        add_sndfile_strings(file, tags.data(), tags.size());

    const auto *float_samples = static_cast<const float *>(samples);
    const auto *int_samples = static_cast<const int32_t *>(samples);
    const uint64_t block_size = 44'100u * 5u;
    for (uint64_t start = 0; start < frames; start += block_size) {
        const uint64_t count = std::min(block_size, frames - start);
        sf_count_t written = 0;
        if (sample_kind == 1) {
            std::vector<short> block(static_cast<size_t>(count) * channels);
            for (size_t i = 0; i < block.size(); ++i)
                block[i] = static_cast<short>(int_samples[start * channels + i]);
            written = sf_writef_short(file, block.data(),
                                      static_cast<sf_count_t>(count));
        } else if (options->encoding == SF_FORMAT_PCM_24 &&
                   sample_kind == 0) {
            written = sf_writef_float(file,
                float_samples + start * channels,
                static_cast<sf_count_t>(count));
        } else {
            written = sf_writef_float(file,
                float_samples + start * channels,
                static_cast<sf_count_t>(count));
        }
        if (written != static_cast<sf_count_t>(count)) {
            const std::string message = sndfile_error(file);
            sf_close(file);
            set_error(error, error_length, message);
            return -5;
        }
    }

    if (type == SF_FORMAT_WAV || type == SF_FORMAT_WAVEX)
        add_sndfile_strings(file, tags.data(), tags.size());

    if (sf_close(file) != 0) {
        set_error(error, error_length, "failed to close PCM export");
        return -6;
    }
    if ((type == SF_FORMAT_AIFF || type == SF_FORMAT_WAV) &&
        !add_id3_chunk(path, info.format, tags.data(), tags.size())) {
        set_error(error, error_length, "failed to write ID3 chunk");
        return -7;
    }
    return 0;
}

extern "C" int aud_export_flac(
    const char *path, uint32_t sample_rate, uint32_t channels,
    const int32_t *samples, uint64_t frames, const void *opaque_options,
    const char *const *names, const char *const *values, size_t tag_count,
    char *error, size_t error_length)
{
    struct FlacOptionsC {
        int bit_depth;
        int level;
    };
    const auto *options = static_cast<const FlacOptionsC *>(opaque_options);
    if (!path || !options || channels == 0 || channels > FLAC__MAX_CHANNELS) {
        set_error(error, error_length, "invalid FLAC export arguments");
        return -1;
    }

    FLAC__StreamEncoder *encoder = FLAC__stream_encoder_new();
    if (!encoder) {
        set_error(error, error_length, "unable to create FLAC encoder");
        return -2;
    }

    const unsigned level = options->level < 0 || options->level > 8
        ? 5u : static_cast<unsigned>(options->level);
    static const bool exhaustive[9] =
        {false, false, false, false, false, false, false, true, true};
    static const bool mid_side[9] =
        {false, true, true, false, true, true, true, true, true};
    static const bool loose_mid_side[9] =
        {false, true, false, false, true, false, false, false, false};
    static const unsigned min_partition[9] =
        {2, 2, 0, 3, 3, 3, 0, 0, 0};
    static const unsigned max_partition[9] =
        {2, 2, 3, 3, 3, 3, 4, 6, 6};
    static const unsigned max_lpc[9] =
        {0, 0, 0, 6, 8, 8, 8, 8, 12};

    bool success =
        FLAC__stream_encoder_set_channels(encoder, channels) &&
        FLAC__stream_encoder_set_sample_rate(encoder, sample_rate);

    std::vector<TagView> tags(tag_count);
    for (size_t i = 0; i < tag_count; ++i)
        tags[i] = {names ? names[i] : nullptr, values ? values[i] : nullptr};
    auto *metadata = make_flac_metadata(tags.data(), tags.size());
    if (!metadata)
        success = false;
    if (success) {
        FLAC__StreamMetadata *metadata_list[] = {metadata};
        success = FLAC__stream_encoder_set_metadata(encoder, metadata_list, 1);
    }

    const unsigned bits = options->bit_depth == 24 ? 24u : 16u;
    success = success &&
        FLAC__stream_encoder_set_bits_per_sample(encoder, bits) &&
        FLAC__stream_encoder_set_do_exhaustive_model_search(
            encoder, exhaustive[level]) &&
        FLAC__stream_encoder_set_do_escape_coding(encoder, false);

    if (channels != 2) {
        success = success &&
            FLAC__stream_encoder_set_do_mid_side_stereo(encoder, false) &&
            FLAC__stream_encoder_set_loose_mid_side_stereo(encoder, false);
    } else {
        success = success &&
            FLAC__stream_encoder_set_do_mid_side_stereo(encoder, mid_side[level]) &&
            FLAC__stream_encoder_set_loose_mid_side_stereo(
                encoder, loose_mid_side[level]);
    }

    success = success &&
        FLAC__stream_encoder_set_qlp_coeff_precision(encoder, 0) &&
        FLAC__stream_encoder_set_min_residual_partition_order(
            encoder, min_partition[level]) &&
        FLAC__stream_encoder_set_max_residual_partition_order(
            encoder, max_partition[level]) &&
        FLAC__stream_encoder_set_rice_parameter_search_dist(encoder, 0) &&
        FLAC__stream_encoder_set_max_lpc_order(encoder, max_lpc[level]);

    if (!success) {
        if (metadata)
            FLAC__metadata_object_delete(metadata);
        FLAC__stream_encoder_delete(encoder);
        set_error(error, error_length, "failed to configure FLAC encoder");
        return -3;
    }

    const FLAC__StreamEncoderInitStatus init_status =
        FLAC__stream_encoder_init_file(encoder, path, nullptr, nullptr);
    if (init_status != FLAC__STREAM_ENCODER_INIT_STATUS_OK) {
        if (metadata)
            FLAC__metadata_object_delete(metadata);
        FLAC__stream_encoder_delete(encoder);
        set_error(error, error_length, "FLAC encoder failed to initialize");
        return -4;
    }
    if (metadata)
        FLAC__metadata_object_delete(metadata);

    constexpr uint64_t block_size = 8192;
    std::vector<std::vector<FLAC__int32>> planar(
        channels, std::vector<FLAC__int32>(block_size));
    std::vector<FLAC__int32 *> pointers(channels);
    for (uint32_t c = 0; c < channels; ++c)
        pointers[c] = planar[c].data();

    bool process_ok = true;
    for (uint64_t start = 0; start < frames && process_ok; start += block_size) {
        const uint64_t count = std::min(block_size, frames - start);
        for (uint64_t i = 0; i < count; ++i)
            for (uint32_t c = 0; c < channels; ++c)
                planar[c][i] = samples[(start + i) * channels + c];
        process_ok = FLAC__stream_encoder_process(
            encoder, pointers.data(), static_cast<unsigned>(count));
    }

    const bool finish_ok = FLAC__stream_encoder_finish(encoder);
    FLAC__stream_encoder_delete(encoder);
    if (!process_ok || !finish_ok) {
        set_error(error, error_length, "failed while encoding FLAC samples");
        return -5;
    }
    return 0;
}

extern "C" int aud_export_wavpack(
    const char *path, uint32_t sample_rate, uint32_t channels,
    const int32_t *samples, uint64_t frames, const void *opaque_options,
    const char *const *names, const char *const *values, size_t tag_count,
    char *error, size_t error_length)
{
    struct WavPackOptionsC {
        int quality;
        int bit_depth;
        int hybrid_mode;
        int create_correction;
        int bit_rate;
    };
    const auto *options = static_cast<const WavPackOptionsC *>(opaque_options);
    if (!path || !options || channels == 0) {
        set_error(error, error_length, "invalid WavPack export arguments");
        return -1;
    }

    FILE *wv = std::fopen(path, "wb+");
    if (!wv) {
        set_error(error, error_length, "unable to open WavPack output");
        return -2;
    }

    std::string correction_path(path);
    correction_path += "c";
    FILE *wvc = nullptr;
    if (options->hybrid_mode && options->create_correction) {
        wvc = std::fopen(correction_path.c_str(), "wb+");
        if (!wvc) {
            std::fclose(wv);
            set_error(error, error_length, "unable to open WavPack correction output");
            return -3;
        }
    } else {
        std::remove(correction_path.c_str());
    }

    WavpackConfig config{};
    config.num_channels = static_cast<int>(channels);
    config.sample_rate = sample_rate;
    config.bits_per_sample = options->bit_depth;
    config.bytes_per_sample = options->bit_depth / 8;
    config.float_norm_exp = options->bit_depth == 32 ? 127 : 0;
    if (channels <= 2)
        config.channel_mask = 0x5 - channels;
    else if (channels <= 18)
        config.channel_mask = (1U << channels) - 1;
    else
        config.channel_mask = 0x3ffff;

    if (options->quality == 0)
        config.flags |= CONFIG_FAST_FLAG;
    else if (options->quality == 2)
        config.flags |= CONFIG_HIGH_FLAG;
    else if (options->quality == 3)
        config.flags |= CONFIG_HIGH_FLAG | CONFIG_VERY_HIGH_FLAG;

    if (options->hybrid_mode) {
        config.flags |= CONFIG_HYBRID_FLAG;
        config.bitrate = static_cast<double>(options->bit_rate) / 10.0;
        if (wvc)
            config.flags |= CONFIG_CREATE_WVC;
    }

    WvWriter writer{wv, nullptr, 0};
    WvWriter cwriter{wvc, nullptr, 0};
    WavpackContext *context = WavpackOpenFileOutput(
        write_wavpack_block, &writer, wvc ? &cwriter : nullptr);
    if (!context ||
        !WavpackSetConfiguration64(context, &config, -1, nullptr) ||
        !WavpackPackInit(context)) {
        const std::string message = context
            ? WavpackGetErrorMessage(context)
            : "unable to create WavPack context";
        if (context)
            WavpackCloseFile(context);
        if (wv)
            std::fclose(wv);
        if (wvc)
            std::fclose(wvc);
        set_error(error, error_length, message);
        return -4;
    }

    constexpr uint64_t block_size = 8192;
    std::vector<int32_t> block(block_size * channels);
    bool process_ok = true;
    for (uint64_t start = 0; start < frames && process_ok; start += block_size) {
        const uint64_t count = std::min(block_size, frames - start);
        const size_t samples_in_block = static_cast<size_t>(count) * channels;
        // Input is frame-interleaved int32; WavpackPackSamples takes a frame
        // count, not a count of individual channel samples.
        std::copy(samples + start * channels,
                  samples + start * channels + samples_in_block,
                  block.begin());
        process_ok = WavpackPackSamples(
            context, block.data(), static_cast<uint32_t>(count)) != 0;
    }

    if (!process_ok || !WavpackFlushSamples(context)) {
        const std::string message = WavpackGetErrorMessage(context);
        WavpackCloseFile(context);
        std::fclose(wv);
        if (wvc)
            std::fclose(wvc);
        set_error(error, error_length, message);
        return -5;
    }

    for (size_t i = 0; i < tag_count; ++i) {
        const char *name = names ? names[i] : nullptr;
        const char *value = values ? values[i] : nullptr;
        if (name && value)
            WavpackAppendTagItem(context, name, value,
                                 static_cast<int>(std::strlen(value)));
    }
    if (!WavpackWriteTag(context)) {
        const std::string message = WavpackGetErrorMessage(context);
        WavpackCloseFile(context);
        std::fclose(wv);
        if (wvc)
            std::fclose(wvc);
        set_error(error, error_length, message);
        return -6;
    }

    std::fflush(wv);
    if (wvc)
        std::fflush(wvc);

    if (writer.first_block_size > 0) {
        std::vector<int32_t> first_block(
            (writer.first_block_size + sizeof(int32_t) - 1) / sizeof(int32_t));
        std::rewind(wv);
        const size_t bytes = std::fread(
            first_block.data(), 1, writer.first_block_size, wv);
        if (bytes != writer.first_block_size) {
            WavpackCloseFile(context);
            std::fclose(wv);
            if (wvc)
                std::fclose(wvc);
            set_error(error, error_length, "unable to update WavPack sample count");
            return -7;
        }
        WavpackUpdateNumSamples(context, first_block.data());
        std::rewind(wv);
        if (std::fwrite(first_block.data(), 1, writer.first_block_size, wv) !=
            writer.first_block_size) {
            WavpackCloseFile(context);
            std::fclose(wv);
            if (wvc)
                std::fclose(wvc);
            set_error(error, error_length, "unable to write WavPack sample count");
            return -8;
        }
    }

    WavpackCloseFile(context);
    const bool close_wv_ok = std::fclose(wv) == 0;
    const bool close_wvc_ok = !wvc || std::fclose(wvc) == 0;
    if (!close_wv_ok || !close_wvc_ok) {
        set_error(error, error_length, "failed to close WavPack output");
        return -9;
    }
    return 0;
}
