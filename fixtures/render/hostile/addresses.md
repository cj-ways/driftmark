# Addresses

- [plain](javascript:alert(1))
- [upper](JAVASCRIPT:alert(2))
- [entity](&#106;avascript:alert(3))
- [named](javascript&colon;alert(4))
- [tab](java&#9;script:alert(5))
- [space]( javascript:alert(6))
- [data](data:text/html,<script>alert(7)</script>)
- [data image](data:image/png;base64,iVBORw0KGgo=)
- [vbscript](vbscript:msgbox(8))
- [file](file:///etc/passwd)
- ![image](javascript:alert(9))
- ![data image](data:image/svg+xml,<svg onload=alert(10)>)
- <a href="javascript:alert(11)">raw</a>
- <a href="&#x6A;avascript:alert(12)">raw entity</a>
- <a href="jav&#x09;ascript:alert(13)">raw tab</a>
- <a href="javascript&colon;alert(14)">raw named</a>
- <a href=" data:text/html,x">raw data</a>
- <img src="data:image/png;base64,AAAA" alt="raw data image">
- <img src="javascript:alert(15)" alt="raw script image">
- [reference][bad]
- <javascript:alert(16)>

[bad]: javascript:alert(17)
