// Minimal JSON reader — just enough to parse `just --dump --dump-format json`.
// Not a general-purpose JSON library: no writer, no comments/trailing-comma
// leniency, no big-number precision handling.
#pragma once

#include <cctype>
#include <cstdint>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

namespace json {

enum class Type { Null, Bool, Number, String, Array, Object };

struct Value {
    Type type = Type::Null;
    bool b = false;
    double num = 0.0;
    std::string str;
    std::vector<Value> arr;
    std::vector<std::pair<std::string, Value>> obj;

    bool is_null() const { return type == Type::Null; }
    bool is_string() const { return type == Type::String; }
    bool is_array() const { return type == Type::Array; }
    bool is_object() const { return type == Type::Object; }

    const Value* find(const std::string& key) const {
        if (type != Type::Object) return nullptr;
        for (auto& kv : obj)
            if (kv.first == key) return &kv.second;
        return nullptr;
    }

    std::string as_string(const std::string& def = "") const {
        return type == Type::String ? str : def;
    }
    bool as_bool(bool def = false) const {
        return type == Type::Bool ? b : def;
    }
};

namespace detail {

class Parser {
public:
    explicit Parser(const std::string& text) : s_(text) {}

    Value parse() {
        skip_ws();
        Value v = parse_value();
        skip_ws();
        return v;
    }

private:
    const std::string& s_;
    size_t i_ = 0;

    [[noreturn]] void fail(const std::string& msg) const {
        throw std::runtime_error("JSON parse error at offset " +
                                  std::to_string(i_) + ": " + msg);
    }

    char peek() const {
        if (i_ >= s_.size()) fail("unexpected end of input");
        return s_[i_];
    }

    char take() {
        if (i_ >= s_.size()) fail("unexpected end of input");
        return s_[i_++];
    }

    void skip_ws() {
        while (i_ < s_.size()) {
            char c = s_[i_];
            if (c == ' ' || c == '\t' || c == '\n' || c == '\r')
                ++i_;
            else
                break;
        }
    }

    bool consume_literal(const char* lit) {
        size_t n = std::char_traits<char>::length(lit);
        if (s_.compare(i_, n, lit) == 0) {
            i_ += n;
            return true;
        }
        return false;
    }

    Value parse_value() {
        skip_ws();
        char c = peek();
        switch (c) {
            case '{': return parse_object();
            case '[': return parse_array();
            case '"': return parse_string_value();
            case 't':
            case 'f': return parse_bool();
            case 'n': return parse_null();
            default: return parse_number();
        }
    }

    Value parse_object() {
        Value v;
        v.type = Type::Object;
        take();  // '{'
        skip_ws();
        if (peek() == '}') {
            take();
            return v;
        }
        while (true) {
            skip_ws();
            if (peek() != '"') fail("expected string key");
            std::string key = parse_string_raw();
            skip_ws();
            if (take() != ':') fail("expected ':'");
            Value val = parse_value();
            v.obj.emplace_back(std::move(key), std::move(val));
            skip_ws();
            char n = take();
            if (n == ',') continue;
            if (n == '}') break;
            fail("expected ',' or '}'");
        }
        return v;
    }

    Value parse_array() {
        Value v;
        v.type = Type::Array;
        take();  // '['
        skip_ws();
        if (peek() == ']') {
            take();
            return v;
        }
        while (true) {
            v.arr.push_back(parse_value());
            skip_ws();
            char n = take();
            if (n == ',') continue;
            if (n == ']') break;
            fail("expected ',' or ']'");
        }
        return v;
    }

    static void append_utf8(std::string& out, uint32_t cp) {
        if (cp <= 0x7F) {
            out += static_cast<char>(cp);
        } else if (cp <= 0x7FF) {
            out += static_cast<char>(0xC0 | (cp >> 6));
            out += static_cast<char>(0x80 | (cp & 0x3F));
        } else if (cp <= 0xFFFF) {
            out += static_cast<char>(0xE0 | (cp >> 12));
            out += static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
            out += static_cast<char>(0x80 | (cp & 0x3F));
        } else {
            out += static_cast<char>(0xF0 | (cp >> 18));
            out += static_cast<char>(0x80 | ((cp >> 12) & 0x3F));
            out += static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
            out += static_cast<char>(0x80 | (cp & 0x3F));
        }
    }

    static int hex_val(char c) {
        if (c >= '0' && c <= '9') return c - '0';
        if (c >= 'a' && c <= 'f') return c - 'a' + 10;
        if (c >= 'A' && c <= 'F') return c - 'A' + 10;
        return -1;
    }

    std::string parse_string_raw() {
        std::string out;
        take();  // opening '"'
        while (true) {
            char c = take();
            if (c == '"') break;
            if (c == '\\') {
                char e = take();
                switch (e) {
                    case '"': out += '"'; break;
                    case '\\': out += '\\'; break;
                    case '/': out += '/'; break;
                    case 'b': out += '\b'; break;
                    case 'f': out += '\f'; break;
                    case 'n': out += '\n'; break;
                    case 'r': out += '\r'; break;
                    case 't': out += '\t'; break;
                    case 'u': {
                        uint32_t cp = 0;
                        for (int k = 0; k < 4; ++k) {
                            int h = hex_val(take());
                            if (h < 0) fail("invalid \\u escape");
                            cp = (cp << 4) | static_cast<uint32_t>(h);
                        }
                        append_utf8(out, cp);
                        break;
                    }
                    default: fail("invalid escape sequence");
                }
            } else {
                out += c;
            }
        }
        return out;
    }

    Value parse_string_value() {
        Value v;
        v.type = Type::String;
        v.str = parse_string_raw();
        return v;
    }

    Value parse_bool() {
        Value v;
        v.type = Type::Bool;
        if (consume_literal("true")) {
            v.b = true;
        } else if (consume_literal("false")) {
            v.b = false;
        } else {
            fail("invalid literal");
        }
        return v;
    }

    Value parse_null() {
        if (!consume_literal("null")) fail("invalid literal");
        return Value{};
    }

    Value parse_number() {
        size_t start = i_;
        if (i_ < s_.size() && s_[i_] == '-') ++i_;
        while (i_ < s_.size() && isdigit(static_cast<unsigned char>(s_[i_]))) ++i_;
        if (i_ < s_.size() && s_[i_] == '.') {
            ++i_;
            while (i_ < s_.size() && isdigit(static_cast<unsigned char>(s_[i_]))) ++i_;
        }
        if (i_ < s_.size() && (s_[i_] == 'e' || s_[i_] == 'E')) {
            ++i_;
            if (i_ < s_.size() && (s_[i_] == '+' || s_[i_] == '-')) ++i_;
            while (i_ < s_.size() && isdigit(static_cast<unsigned char>(s_[i_]))) ++i_;
        }
        if (i_ == start) fail("invalid number");
        Value v;
        v.type = Type::Number;
        v.num = std::stod(s_.substr(start, i_ - start));
        return v;
    }
};

}  // namespace detail

inline Value parse(const std::string& text) {
    detail::Parser p(text);
    return p.parse();
}

}  // namespace json
