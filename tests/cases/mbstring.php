<?php
$s = "Héllo Wörld ñ 日本語テキスト 🎉";
$bad = "a\xffb\xe2\x82c\xf0\x9f\x98";
var_dump(mb_internal_encoding(), mb_strlen($s), mb_strlen($bad), mb_strlen($s, '8bit'), mb_strlen("abc", 'ASCII'));
var_dump(mb_substr($s, 6, 5), mb_substr($s, -4), mb_substr($s, 2, -3), mb_substr($s, 100), mb_substr($bad, 1, 3), mb_substr($s, -100, 2));
var_dump(mb_strpos($s, 'ö'), mb_strpos($s, 'x'), mb_strrpos($s, 'l'), mb_stripos($s, 'WÖR'), mb_strripos($s, 'L'), mb_strpos($s, 'l', 4), mb_strpos($s, 'l', -3));
var_dump(mb_strtoupper($s), mb_strtolower("ÀÉÎÕÜ ΣΑΣ İ"), mb_strtoupper("straße ŉ"), mb_convert_case("hello wORLD o'neil x-y", MB_CASE_TITLE), mb_convert_case("ǅ", MB_CASE_LOWER), mb_convert_case("Straße", MB_CASE_FOLD));
var_dump(mb_check_encoding($s), mb_check_encoding($bad), mb_check_encoding($s, 'ASCII'), mb_check_encoding("abc", 'ASCII'));
var_dump(bin2hex(mb_convert_encoding("café", 'ISO-8859-1', 'UTF-8')), mb_convert_encoding("caf\xe9", 'UTF-8', 'ISO-8859-1'), mb_convert_encoding($bad, 'UTF-8', 'UTF-8'), bin2hex(mb_convert_encoding("aé", 'UTF-16BE')), bin2hex(mb_convert_encoding("aé", 'UTF-16LE', 'UTF-8')), mb_convert_encoding("\x00a\x00\xe9", 'UTF-8', 'UTF-16'), mb_convert_encoding("日本", 'ASCII'), mb_convert_encoding("\x80\x9f", 'UTF-8', 'Windows-1252'));
var_dump(mb_detect_encoding("abc"), mb_detect_encoding("café"), mb_detect_encoding("caf\xe9", ['UTF-8', 'ISO-8859-1']), mb_detect_encoding("caf\xe9", 'UTF-8', true), mb_detect_encoding("abc", 'UTF-8, ASCII'));
var_dump(mb_substitute_character(), mb_substitute_character(0x3f), mb_substitute_character('none'), mb_convert_encoding($bad, 'UTF-8', 'UTF-8'), mb_substitute_character('long'), mb_convert_encoding("日", 'ASCII'), mb_substitute_character(0xFFFD));
var_dump(mb_str_split("añb日", 2), mb_ord("日"), mb_chr(0x1F389), mb_chr(0xD800), mb_strwidth("日本abc"), mb_strimwidth("Hello 日本語", 0, 8, "…"), mb_substr_count("ababab", "ab"));
var_dump(mb_strstr("user@example.com", "@"), mb_strstr("user@example.com", "@", true), mb_strrchr("a/b/c", "/"), mb_stristr("HeLLo", "ll"), mb_scrub($bad), mb_str_pad("日", 4, "ab", STR_PAD_BOTH), mb_trim("  日本 \u{3000}"), mb_ucfirst("élan"), mb_lcfirst("Élan"));
var_dump(mb_strcut("日本語", 1, 4), mb_preferred_mime_name("sjis"), in_array('UTF-8', mb_list_encodings()), mb_encode_numericentity("日a", [0x80, 0x10ffff, 0, 0x1fffff], 'UTF-8'), mb_decode_numericentity("&#26085;a&#x41;", [0x0, 0x10ffff, 0, 0x1fffff]));
try { mb_strlen("x", "nope"); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }
var_dump(mb_language(), mb_http_output(), mb_detect_order(), mb_get_info('internal_encoding'), mb_regex_encoding());
