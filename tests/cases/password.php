<?php
$t = microtime(true);
foreach (['', 'password', 'pässwörd', str_repeat('x', 80)] as $pw) {
    echo crypt($pw, '$2y$04$abcdefghijklmnopqrstuu'), "\n";
    echo crypt($pw, '$2y$05$0123456789ABCDEFGHIJKL'), "\n";
}
$h = password_hash('secret', PASSWORD_DEFAULT, ['cost' => 5]);
var_dump(strlen($h), substr($h, 0, 7), password_verify('secret', $h), password_verify('wrong', $h), password_needs_rehash($h, PASSWORD_BCRYPT), password_needs_rehash($h, PASSWORD_BCRYPT, ['cost' => 5]), password_get_info($h));
var_dump(password_verify('rasmuslerdorf', '$2y$10$.vGA1O9wmRjrwAVXD98HNOgsNpDczlqm3Jq7KnEd1rVAGv3Fykk1a'));
