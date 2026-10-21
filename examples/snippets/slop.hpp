// Slop for C++ games (Unreal, or your own engine): saves, mods and asking the Slop app for a
// server's mods. C++17, standard library only. Outside the Slop app it falls back: your own
// save folder, no mods, and request_mods answers "no_link".
#pragma once
#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <optional>
#include <sstream>
#include <string>
#include <thread>
#include <utility>
#include <vector>

namespace slop {
namespace fs = std::filesystem;

inline std::string env(const char* name) {
    const char* v = std::getenv(name);
    return v ? v : "";
}

// Where saves, settings and logs go: Slop's save folder, else `fallback`.
inline fs::path save_dir(const fs::path& fallback) {
    std::string dir = env("SLOP_SAVE_DIR");
    fs::path path = dir.empty() ? fallback : fs::u8path(dir);
    fs::create_directories(path);
    return path;
}

// Saves safely: a new file first, renamed over the old one.
inline bool save(const fs::path& fallback, const std::string& name, const std::string& data) {
    fs::path path = save_dir(fallback) / name, tmp = path;
    tmp += ".tmp";
    {
        std::ofstream out(tmp, std::ios::binary);
        if (!(out << data)) return false;
    }
    std::error_code ec;
    fs::rename(tmp, path, ec);
    return !ec;
}

// A save, or nothing when it's missing or over 1 MiB.
inline std::optional<std::string> load(const fs::path& fallback, const std::string& name) {
    fs::path path = save_dir(fallback) / name;
    std::error_code ec;
    auto size = fs::file_size(path, ec);
    if (ec || size > 1024 * 1024) return std::nullopt;
    std::ifstream in(path, std::ios::binary);
    std::stringstream buf;
    buf << in.rdbuf();
    return buf.str();
}

// Every mod folder: your game's own mods folder (if you named one on your store page), then
// SLOP_MODS. Each holds one mod from Slop.
inline std::vector<fs::path> mod_dirs(const fs::path& own_folder = {}) {
    std::vector<fs::path> found;
    std::error_code ec;
    if (!own_folder.empty() && fs::is_directory(own_folder, ec)) {
        for (auto& e : fs::directory_iterator(own_folder, ec))
            if (e.is_directory()) found.push_back(e.path());
        std::sort(found.begin(), found.end());
    }
    auto address = [](const fs::path& p) {
        return p.filename() == "files" ? p.parent_path().filename() : p.filename();
    };
#ifdef _WIN32
    const char sep = ';';
#else
    const char sep = ':';
#endif
    std::stringstream list(env("SLOP_MODS"));
    for (std::string item; std::getline(list, item, sep);) {
        if (item.empty()) continue;
        fs::path dir = fs::u8path(item);
        bool known = false;
        for (auto& f : found) known = known || address(f) == address(dir);
        if (!known) found.push_back(dir);
    }
    return found;
}

inline std::string quote(const std::string& s) {
    std::string out = "\"";
    for (char c : s) {
        if (c == '"' || c == '\\') { out += '\\'; out += c; }
        else if (static_cast<unsigned char>(c) < 0x20) { char b[8]; std::snprintf(b, sizeof b, "\\u%04x", c); out += b; }
        else out += c;
    }
    return out + "\"";
}

// An answer's "status": done, declined, failed, waiting, no_link or timeout.
inline std::string status(const std::string& answer) {
    auto at = answer.find("\"status\"");
    if (at == std::string::npos) return "";
    auto start = answer.find('"', answer.find(':', at) + 1);
    auto end = answer.find('"', start + 1);
    return start == std::string::npos || end == std::string::npos ? "" : answer.substr(start + 1, end - start - 1);
}

// Asks the Slop app for mods by address and version ("" for any). The player confirms. Returns
// the answer's JSON (see status()). Blocks: call it on a worker thread.
inline std::string request_mods(const std::vector<std::pair<std::string, std::string>>& mods,
                                const std::string& for_whom, std::chrono::seconds give_up_after = std::chrono::seconds(600)) {
    std::string link = env("SLOP_LINK");
    if (link.empty()) return "{\"status\":\"no_link\"}";
    fs::path dir = fs::u8path(link);
    std::string name = "r" + std::to_string(std::chrono::steady_clock::now().time_since_epoch().count());
    std::string body = "{\"mods\":[";
    for (size_t i = 0; i < mods.size(); ++i) {
        body += (i ? "," : "") + std::string("{\"slug\":") + quote(mods[i].first);
        if (!mods[i].second.empty()) body += ",\"version\":" + quote(mods[i].second);
        body += "}";
    }
    body += "],\"for\":" + quote(for_whom) + "}";
    fs::path tmp = dir / "requests" / (name + ".tmp");
    { std::ofstream out(tmp, std::ios::binary); out << body; }
    std::error_code ec;
    fs::rename(tmp, dir / "requests" / (name + ".json"), ec);
    if (ec) return "{\"status\":\"failed\",\"message\":\"couldn't write the request\"}";
    fs::path answer_path = dir / "answers" / (name + ".json");
    auto until = std::chrono::steady_clock::now() + give_up_after;
    while (std::chrono::steady_clock::now() < until) {
        std::this_thread::sleep_for(std::chrono::milliseconds(500));
        std::ifstream in(answer_path, std::ios::binary);
        if (!in) continue;
        std::stringstream buf;
        buf << in.rdbuf();
        std::string s = status(buf.str());
        if (!s.empty() && s != "waiting") return buf.str();
    }
    return "{\"status\":\"timeout\"}";
}

}  // namespace slop
