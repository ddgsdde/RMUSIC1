// Keep the SDK's platform ABI and packed structures inside C++.
#include <cstdint>
#include "asiosys.h"
#include "asio.h"

extern "C" int rmusic_asio_io_format(int native_dsd) {
    ASIOIoFormat format{};
    format.FormatType = native_dsd ? kASIODSDFormat : kASIOPCMFormat;
    if (native_dsd) {
        const auto error = ASIOFuture(kAsioCanDoIoFormat, &format);
        if (error != ASE_SUCCESS || format.FormatType != kASIODSDFormat)
            return error == ASE_SUCCESS || error == ASE_OK ? ASE_InvalidMode : error;
    }
    auto error = ASIOFuture(kAsioSetIoFormat, &format);
    if (!native_dsd && error == ASE_NotPresent) return ASE_OK;
    if (error != ASE_SUCCESS || format.FormatType != (native_dsd ? kASIODSDFormat : kASIOPCMFormat))
        return error == ASE_SUCCESS || error == ASE_OK ? ASE_InvalidMode : error;
    error = ASIOFuture(kAsioGetIoFormat, &format);
    if (error != ASE_SUCCESS || format.FormatType != (native_dsd ? kASIODSDFormat : kASIOPCMFormat))
        return error == ASE_SUCCESS || error == ASE_OK ? ASE_InvalidMode : error;
    return ASE_OK;
}
extern "C" int rmusic_asio_set_rate(double rate) {
    return ASIOSetSampleRate(rate);
}
extern "C" int rmusic_asio_position(uint64_t* frames, uint64_t* nanos) {
    ASIOSamples position{};
    ASIOTimeStamp stamp{};
    const auto error = ASIOGetSamplePosition(&position, &stamp);
    if (error == ASE_OK) {
        *frames = (uint64_t(position.hi) << 32) | position.lo;
        *nanos = (uint64_t(stamp.hi) << 32) | stamp.lo;
    }
    return error;
}
extern "C" int rmusic_asio_channel_type(int channel, int* type) {
    ASIOChannelInfo info{};
    info.channel = channel;
    const auto error = ASIOGetChannelInfo(&info);
    if (error == ASE_OK) *type = info.type;
    return error;
}
