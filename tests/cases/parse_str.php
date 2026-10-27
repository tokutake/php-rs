<?php
foreach (["a=1&b[]=2&b[]=3", "x.y=1&x y=2&  lead=3", "arr[k][j]=v&arr[k][]=w&arr[]=z", "a[b=1&c]=2&d[e]f=3", "=nokey&novalue&e=&f=%20%2B+x", "a[0]=1&a[0]=2&a[x]=3", "q=a%26b&r=%ZZ", "dup=1&dup=2", "", "a[][]=1&a[][]=2", "&&&x=1&&", "a.b[c.d]=1", "k%5B1%5D=v", "[x]=1&]=2", "a[ b ]=1"] as $q) {
    parse_str($q, $r);
    echo json_encode($q), " => ", json_encode($r), "\n";
}
