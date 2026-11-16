// From oven-sh/bun src/jsc/bindings/BunAOTCompile.cpp (MIT) at the pinned frontend revision:
// the engine calls of the front end, against the redjs JavaScriptCore prebuilt.
#include "config.h"
#include <wtf/text/MakeString.h>

#if ENABLE(AOT)

#include "compile.h"

#include "AOTSchemaHash.h"
#include <JavaScriptCore/AOTBuiltinTableBindings.h>
#include <JavaScriptCore/AOTCompileOptionWords.h>
#include <JavaScriptCore/AOTCurrentWork.h>
#include <JavaScriptCore/AOTDriver.h>
#include <JavaScriptCore/AOTOptionWords.h>
#include <JavaScriptCore/AOTRegExpSummary.h>
#include <JavaScriptCore/AOTRemovableFeature.h>
#include <JavaScriptCore/AOTRuntimeDescription.h>
#include <JavaScriptCore/AOTStoredStrings.h>
#include <JavaScriptCore/AOTThreads.h>
#include <JavaScriptCore/AOTWireDiagnosticColumns.h>
#include <JavaScriptCore/YarrErrorCode.h>
#include <JavaScriptCore/YarrFlags.h>
#include <JavaScriptCore/YarrSyntaxChecker.h>
#include <algorithm>
#include <cstring>
#include <memory>
#include <optional>
#include <span>
#include <wtf/MainThread.h>
#include <wtf/OptionSet.h>
#include <wtf/Threading.h>
#include <wtf/text/CString.h>
#include <wtf/text/StringBuilder.h>
#include <wtf/text/StringView.h>
#include <wtf/text/WTFString.h>

#if OS(WINDOWS)
#include <fcntl.h>
#include <io.h>
#include <stdlib.h>
#endif

static_assert(JSC::AOT::Wire::schemaHash == BUN_AOT_SCHEMA_HASH, "the Rust half of the compiler and JavaScriptCore's AOTWire.h were generated from different schemas");
static_assert(sizeof(JSC::AOT::Wire::ResultSection) == sizeof(uint32_t));

namespace Wire = JSC::AOT::Wire;

static constexpr BunAOTBytes noBytes { nullptr, 0 };

struct BunAOTResult {
    WTF_DEPRECATED_MAKE_STRUCT_FAST_ALLOCATED(BunAOTResult);

    std::unique_ptr<JSC::AOT::CompileOutput> output;
};

extern "C" BunAOTBytes Bun__AOT__resultSection(const BunAOTResult* result, Wire::ResultSection section)
{
    if (static_cast<uint32_t>(section) >= JSC::AOT::numberOfResultSections)
        return noBytes;
    std::span<const uint8_t> bytes = result->output->section(section);
    return { bytes.data(), bytes.size() };
}

extern "C" void Bun__AOT__freeResult(BunAOTResult* result)
{
    delete result;
}

static String stringFromUTF8(BunAOTBytes bytes)
{
    return String::fromUTF8ReplacingInvalidSequences(std::span { bytes.ptr, bytes.len });
}

extern "C" void Bun__AOT__initializeOptions(Wire::CompileOptions* options)
{
    WTF::initialize();

    *options = JSC::AOT::makeDefaultCompileOptions();
}

extern "C" size_t Bun__AOT__applyOptionWord(Wire::CompileOptions* options, BunAOTBytes word, uint8_t* message, size_t capacity)
{
    WTF::initialize();

    Wire::CompileOptions changedOptions = *options;
    auto error = JSC::AOT::applyOptionWord(changedOptions, stringFromUTF8(word));
    if (!error) {
        *options = changedOptions;
        return 0;
    }

    UTF8CString utf8 = error->utf8();
    RELEASE_ASSERT(utf8.length());
    if (size_t copiedLength = std::min(capacity, utf8.length()))
        memcpy(message, utf8.data(), copiedLength);
    return utf8.length();
}

// The engine run that answers the front end's flow questions, exactly as in Bun. Its image is
// not used: redjs's compiler makes the image from the IR file, with the options its loader checks.
// Bun's Buffer encoders are not linked: the compiler then leaves Buffer strings to run time.
extern "C" int32_t rjs_init(const char*);

extern "C" BunAOTResult* Bun__AOT__compile(BunAOTBytes program, const JSC::AOT::ExternalFunctionBodies* externalBodies, void (*releaseProgram)(void* owner), void (*releaseBytesOfProgram)(void* owner), void* programOwner, const Wire::CompileOptions* options, const JSC::AOT::HostThreadPool* threadPool, size_t (*functionSourceSize)(void* owner, uint32_t module), void (*copyFunctionSource)(void* owner, uint32_t module, uint8_t* destination), const uint8_t* (*viewFunctionSource)(void* owner, uint32_t module), void (*releaseFunctionSources)(void* owner), BunAOTBytes orderFile)
{
#if OS(WINDOWS)
    _setmode(2, _O_BINARY);
    _set_fmode(_O_BINARY);
#endif

    if (rjs_init(nullptr)) {
        fprintf(stderr, "sk-aot: the engine did not initialize\n");
        abort();
    }

    auto* result = new BunAOTResult;
    result->output = JSC::AOT::compileProgram(std::span { program.ptr, program.len }, *options, threadPool, JSC::AOT::ProgramOwner { releaseProgram, programOwner, externalBodies, { programOwner, functionSourceSize, copyFunctionSource, viewFunctionSource, releaseFunctionSources }, releaseBytesOfProgram, { }, { orderFile.ptr, orderFile.len } });
    return result;
}

extern "C" size_t Bun__AOT__describeDifferenceOfRuntime(const uint8_t* description, size_t size, uint8_t* message, size_t capacity)
{
    std::optional<JSC::AOT::RuntimeDifference> difference = JSC::AOT::findRuntimeDifference({ description, size });
    if (!difference)
        return 0;

    WTF::initialize();
    String sentence = difference->name.isNull()
        ? "its description of the run time can't be read"_str
        : makeString(difference->name, " is "_s, difference->there, " there and "_s, difference->here, " here"_s);
    UTF8CString utf8 = sentence.utf8();
    if (size_t copiedLength = std::min(capacity, utf8.length()))
        memcpy(message, utf8.data(), copiedLength);
    return utf8.length();
}

extern "C" size_t Bun__AOT__imageAlignment()
{
    return JSC::AOT::imageAlignment();
}

extern "C" void Bun__AOT__relocateImage(uint8_t* image, size_t size, uint64_t address)
{
    JSC::AOT::relocateImage({ image, size }, address);
}

extern "C" uint32_t Bun__AOT__readStoredStrings(const uint8_t* file, size_t size, uint8_t form, void* stored, void (*take)(void* stored, const uint8_t* strings, size_t sizeOfStrings, const uint8_t* report, size_t sizeOfReport))
{
    JSC::AOT::StoredStrings read = JSC::AOT::readStoredStrings({ file, size }, static_cast<JSC::AOT::StoredStringsForm>(form));
    take(stored, read.strings.span().data(), read.strings.size(), read.report.span().data(), read.report.size());
    return static_cast<uint32_t>(read.verdict);
}

extern "C" size_t Bun__AOT__peakResidentMegabytesOfProcess()
{
    return JSC::AOT::processPeakResidentMegabytes();
}

extern "C" size_t Bun__AOT__describeWhereThisThreadStopped(uint8_t* buffer, size_t capacity)
{
    return JSC::AOT::describeCurrentThreadWork({ reinterpret_cast<char*>(buffer), capacity });
}

extern "C" size_t Bun__AOT__describeWhatThreadsAreCompiling(uint8_t* buffer, size_t capacity)
{
    return JSC::AOT::describeAllThreadsWork({ reinterpret_cast<char*>(buffer), capacity });
}

// What the parser of JavaScript says of a regular expression literal, which it refuses the whole file for. How deep a pattern may be nested depends on the thread, so that is left to where the literal is evaluated. The front end reads the flags itself.
extern "C" size_t Bun__AOT__describeRegExpSyntaxError(const uint8_t* pattern, size_t patternSize, const uint8_t* flags, size_t flagsSize, uint8_t* message, size_t capacity)
{
    WTF::initialize();
    String patternText = String::fromUTF8ReplacingInvalidSequences(std::span { pattern, patternSize });
    String flagsText = String::fromUTF8ReplacingInvalidSequences(std::span { flags, flagsSize });
    JSC::Yarr::ErrorCode error = JSC::Yarr::checkSyntax(patternText, flagsText);
    if (!JSC::Yarr::hasHardError(error) || error == JSC::Yarr::ErrorCode::InvalidRegularExpressionFlags)
        return 0;

    ASCIILiteral sentence = JSC::Yarr::errorMessage(error);
    if (size_t copiedLength = std::min(capacity, sentence.length()))
        memcpy(message, sentence.characters(), copiedLength);
    return sentence.length();
}

static JSC::Yarr::ErrorCode errorOfRegExpPattern(StringView pattern, OptionSet<JSC::Yarr::Flags> flags)
{
    JSC::AOT::RegExpSummary summary = JSC::AOT::summarizeRegExp(pattern, flags);
    if (summary.canBeCreated())
        return JSC::Yarr::ErrorCode::NoError;
    if (JSC::Yarr::hasHardError(summary.syntaxError))
        return summary.syntaxError;
    if (JSC::Yarr::hasHardError(summary.constructionError))
        return summary.constructionError;
    return JSC::Yarr::ErrorCode::TooManyDisjunctions;
}

// What stands against creating the literal /pattern/flags, whose flags are valid, from its record. Both texts come as they do for Bun__AOT__describeRegExpSyntaxError.
// A hard error: its pattern can never be built. TooManyDisjunctions: it is nested too deep for the build to analyze.
static JSC::Yarr::ErrorCode errorOfRegExpLiteral(const uint8_t* pattern, size_t patternSize, const uint8_t* flags, size_t flagsSize)
{
    WTF::initialize();
    String patternText = String::fromUTF8ReplacingInvalidSequences(std::span { pattern, patternSize });
    String flagsText = String::fromUTF8ReplacingInvalidSequences(std::span { flags, flagsSize });
    std::optional<OptionSet<JSC::Yarr::Flags>> parsedFlags = JSC::Yarr::parseFlags(flagsText);
    RELEASE_ASSERT(parsedFlags);

    // The literal may stand deep in an expression, and with assertions or ASAN the analysis needs more stack than a thread of the pool is promised.
    if (JSC::AOT::hasStackForRegExpCompiler())
        return errorOfRegExpPattern(patternText, *parsedFlags);
    JSC::Yarr::ErrorCode error = JSC::Yarr::ErrorCode::NoError;
    JSC::AOT::runOnBuildThread([&] {
        error = errorOfRegExpPattern(patternText, *parsedFlags);
    });
    return error;
}

// 0: from its record, which cannot throw. 1: its pattern can never be built. 2: whether it can be built depends on the stack of the thread that evaluates it.
extern "C" uint32_t Bun__AOT__classifyRegExpLiteral(const uint8_t* pattern, size_t patternSize, const uint8_t* flags, size_t flagsSize)
{
    JSC::Yarr::ErrorCode error = errorOfRegExpLiteral(pattern, patternSize, flags, flagsSize);
    if (!JSC::Yarr::hasError(error))
        return 0;
    return JSC::Yarr::hasHardError(error) ? 1 : 2;
}

// The message of the SyntaxError that a literal of class 1 throws wherever it is evaluated. Nothing for the other two.
extern "C" size_t Bun__AOT__describeRegExpConstructionError(const uint8_t* pattern, size_t patternSize, const uint8_t* flags, size_t flagsSize, uint8_t* message, size_t capacity)
{
    JSC::Yarr::ErrorCode error = errorOfRegExpLiteral(pattern, patternSize, flags, flagsSize);
    if (!JSC::Yarr::hasHardError(error))
        return 0;

    ASCIILiteral sentence = JSC::Yarr::errorMessage(error);
    if (size_t copiedLength = std::min(capacity, sentence.length()))
        memcpy(message, sentence.characters(), copiedLength);
    return sentence.length();
}

extern "C" bool Bun__AOT__hasCompiler()
{
    return true;
}

// redjs's rjs_aot_compile with option words applied after its options, for probes of engine options.
struct SkAotImage {
    std::unique_ptr<JSC::AOT::CompileOutput> output;
    UTF8CString diagnostic;
};

static bool compiledWithoutErrors(const JSC::AOT::CompileOutput& output)
{
    if (!output.hasImage() || !output.section(Wire::ResultSection::InternalErrors).empty())
        return false;
    for (const auto& record : output.records<Wire::UserDiagnosticRecord>(Wire::ResultSection::UserDiagnostics)) {
        if (Wire::severityOf(record.kind) == Wire::DiagnosticSeverity::Error)
            return false;
    }
    return true;
}

extern "C" SkAotImage* sk_aot_compile_image(const uint8_t* program, size_t length, const uint8_t* identity, const BunAOTBytes* words, size_t wordCount)
{
    if (rjs_init(nullptr))
        return nullptr;
    // The front end puts the options it compiled with behind the program.
    if (length > sizeof(Wire::CompileOptions)) {
        Wire::CompileOptions behind;
        std::memcpy(&behind, program + length - sizeof(behind), sizeof(behind));
        if (behind.size == sizeof(behind))
            length -= sizeof(behind);
    }
    auto image = std::make_unique<SkAotImage>();
    auto options = JSC::AOT::makeDefaultCompileOptions();
    options.target.referenceWidth = Wire::ReferenceWidth::Narrow;
    options.target.imageForm = Wire::ImageForm::PositionIndependent;
    options.threadCount = 1;
    std::memcpy(options.executableIdentity.data(), identity, 16);
    StringBuilder messages;
    for (size_t index = 0; index < wordCount; ++index) {
        if (auto error = JSC::AOT::applyOptionWord(options, stringFromUTF8(words[index]))) {
            image->diagnostic = error->utf8();
            return image.release();
        }
    }
    image->output = JSC::AOT::compileProgram({ program, length }, options);
    if (!image->output)
        messages.append("native compiler returned no output"_s);
    else {
        const auto& output = *image->output;
        auto text = output.section(Wire::ResultSection::Text);
        auto append = [&](Wire::TextRange range) {
            if (range.offset <= text.size() && range.length <= text.size() - range.offset)
                messages.append(String::fromUTF8ReplacingInvalidSequences(text.subspan(range.offset, range.length)));
            messages.append('\n');
        };
        for (const auto& record : output.records<Wire::UserDiagnosticRecord>(Wire::ResultSection::UserDiagnostics))
            append(record.message);
        for (const auto& record : output.records<Wire::InternalErrorRecord>(Wire::ResultSection::InternalErrors))
            append(record.message);
    }
    image->diagnostic = messages.toString().utf8();
    return image.release();
}

extern "C" size_t sk_aot_image_bytes(SkAotImage* image, const uint8_t** bytes)
{
    if (!image->output || !compiledWithoutErrors(*image->output))
        return 0;
    auto section = image->output->section(Wire::ResultSection::Image);
    *bytes = section.data();
    return section.size();
}

extern "C" size_t sk_aot_image_entries(SkAotImage* image, const uint32_t** entries)
{
    if (!image->output)
        return 0;
    auto ids = image->output->records<uint32_t>(Wire::ResultSection::EntryModuleIds);
    *entries = ids.data();
    return ids.size();
}

extern "C" size_t sk_aot_image_diagnostic(SkAotImage* image, const char** bytes)
{
    *bytes = reinterpret_cast<const char*>(image->diagnostic.data());
    return image->diagnostic.length();
}

extern "C" void sk_aot_image_free(SkAotImage* image)
{
    delete image;
}
#else
#error "sk-aot needs the JavaScriptCore prebuilt with ENABLE(AOT)"
#endif
