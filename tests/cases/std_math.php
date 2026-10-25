<?php
var_dump(abs(-5), abs(-5.5), abs(PHP_INT_MIN), abs("-3"), ceil(4.1), ceil(-4.1), floor(4.9), floor(-4.9), floor(5));
var_dump(round(2.5), round(-2.5), round(3.14159, 2), round(1234.5678, -2), round(1.955, 2), round(5.045, 2), round(5.055, 2), round(-0.4), round(7, -1), round(1.45, 1), round(0.285, 2));
var_dump(round(2.5, 0, PHP_ROUND_HALF_EVEN), round(3.5, 0, PHP_ROUND_HALF_EVEN), round(2.5, 0, PHP_ROUND_HALF_DOWN), round(-2.5, 0, PHP_ROUND_HALF_ODD));
var_dump(sqrt(16), sqrt(2), sqrt(-1), pow(2, 10), pow(2, -1), pow(2.0, 3), pow("3", 2), 2 ** 63, intdiv(7, 2), fmod(10, 3), fmod(-7.5, 2), fdiv(1, 0), fdiv(-1, 0), is_nan(fdiv(0, 0)));
var_dump(max(1, 3, 2), max([4, 8, 6]), max("apple", "banana"), max([1, 2], [1, 3]), max(1, "2", 3.5), min(2, 1.5, "3"), min([0, -1]), max("10", "9"), max("abc", 0), min("abc", 0));
var_dump(exp(1), log(M_E), log(8, 2), log10(1000), log1p(0), expm1(0), sin(0), cos(0), tan(0), atan2(1, 1), pi(), hypot(3, 4), deg2rad(180), rad2deg(M_PI));
var_dump(is_finite(1.0), is_infinite(INF), is_infinite(-INF), is_nan(NAN), is_finite(NAN));
var_dump(decbin(10), bindec("1010"), decoct(8), octdec("777"), dechex(255), hexdec("1A"), base_convert("ff", 16, 2), base_convert("zz", 36, 10), dechex(-1), bindec("2"), hexdec("xyz1"));
var_dump(array_sum([1, 2.5, "3"]), array_product([2, "3", 4]), array_product([]), number_format(1234.5), number_format(-0.5), number_format(1e15, 2), number_format(0.129, 2), number_format(1234.5678, 2, '.', ' '), number_format(12345678, 0, '', "'"));
var_dump(intdiv(-7, 2), 7 % -3, -7 % 3, fmod(7, -3), abs(-0.0), max([]) ?? null);
