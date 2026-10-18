# Elements that run code

<script>alert(1)</script>

<SCRIPT SRC="https://example.com/x.js"></SCRIPT>

Inline <script>alert(2)</script> and <style>body{display:none}</style> tags.

<iframe src="https://example.com"></iframe>

<object data="x.swf"></object><embed src="x.swf">

<svg onload="alert(3)"><circle r="1"/></svg>

<math><mi>x</mi></math>

<template><img src=x onerror=alert(4)></template>

<textarea><script>alert(5)</script></textarea>

<title><script>alert(6)</script></title>

<noscript><p title="</noscript><img src=x onerror=alert(7)>"></noscript>

<xmp><script>alert(8)</script></xmp>

<form action="https://example.com"><input name="q"><button>Go</button></form>

<base href="https://example.com/"><meta http-equiv="refresh" content="0;url=https://example.com"><link rel="stylesheet" href="x.css">

<plaintext>everything after this would be text
