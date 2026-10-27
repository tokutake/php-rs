<?php
$samples = ["&amp; &lt;b&gt; &quot;q&quot; &#039;s&#39; &apos; &hellip; &eacute;&copy;&nbsp;x", "&#65;&#x42;&#X43; &#0; &#1; &#128; &#xD800; &#x110000; &#9; &#13;", "&notanentity; &amp &; &#; &#x; &AMP; &NotEqualTilde; &nesim; &bne;", "no entities here", "&", "&a;", "&lt"];
foreach ($samples as $s) {
    foreach ([ENT_QUOTES, ENT_COMPAT, ENT_NOQUOTES, ENT_QUOTES | ENT_HTML5, ENT_QUOTES | ENT_XML1, ENT_QUOTES | ENT_XHTML] as $f) {
        echo bin2hex(html_entity_decode($s, $f)), "\n";
    }
    echo bin2hex(html_entity_decode($s)), "\n";
}
foreach (["<a href='x'>Café & \"bar\" © ™ …</a>", "≂̸ ≠ fj", "bad \xff byte", "&amp; &copy; &#123; &nope; & x"] as $s) {
    foreach ([ENT_QUOTES, ENT_COMPAT | ENT_HTML5, ENT_QUOTES | ENT_XML1, ENT_IGNORE, ENT_NOQUOTES | ENT_SUBSTITUTE] as $f) {
        echo htmlentities($s, $f), "\n";
        echo htmlentities($s, $f, 'UTF-8', false), "\n";
    }
}
